//! Explicit `PostgreSQL` migration runner and read-only schema verifier.

use std::collections::BTreeSet;

use postgres::error::SqlState;
use postgres::types::FromSqlOwned;
use postgres::{Client, GenericClient, IsolationLevel, Row};
use sha2::{Digest, Sha256};

use crate::migrations::{
    DatabaseRole, ManifestEvidence, MigrationDescriptor, MigrationStatus, MigrationTarget,
    POSTGRES_SCHEMA_VERSION, PostgresStoreSetupError, PostgresStoreSetupErrorKind,
    STORE_V2_SCHEMA_VERSION, SUPPORTED_POSTGRES_MAJOR, Sha256Hex, migration_manifest,
    verify_embedded_manifest, verify_v1_manifest_prefix, verify_v2_manifest_prefix,
    verify_v3_manifest_prefix,
};

const MIGRATION_ADVISORY_LOCK: i64 = 0x4c41_5454_4943_4501;
const REQUIRED_APPLICATION_NAME: &str = "lattice-devos-task019";
const CATALOG_SIGNATURE_DOMAIN: &[u8] = b"LATTICE_POSTGRES_CATALOG_SIGNATURE_V1\0";
const V1_EXPECTED_RELATION_SIGNATURE: &str =
    "53873b1f2624cab0e03aa257bcdff1e6ace1c3ba215958596dc5257ba0ea24eb";
const V1_EXPECTED_COLUMN_SIGNATURE: &str =
    "b4d90e0356331d65d425327d38e60faabe1c06e8b126a4590c7d3bea9dd34c35";
const V1_EXPECTED_CONSTRAINT_SIGNATURE: &str =
    "10f11fb138749f97959016aaf46773b641a0c334d3e9cb3e706eff86b512adb9";
const V1_EXPECTED_INDEX_SIGNATURE: &str =
    "2e2fdc09b56753defc5c202f70317f7f1f1246d6f81fbe35963a22feb5dbfd94";
const V2_EXPECTED_RELATION_SIGNATURE: &str = V1_EXPECTED_RELATION_SIGNATURE;
const V2_EXPECTED_COLUMN_SIGNATURE: &str =
    "945babc960c852d3d313483770f787a12759f986e2678795128427664377b623";
const V2_EXPECTED_CONSTRAINT_SIGNATURE: &str =
    "9cb7ae1273ee1096891f4c30e57fb5ef8140505a37f36b93ec6d9e115abac800";
const V2_EXPECTED_INDEX_SIGNATURE: &str = V1_EXPECTED_INDEX_SIGNATURE;
const V3_EXPECTED_RELATION_SIGNATURE: &str =
    "85619233866577a32550fac8f83f9995c05f24ddeaaf64d9563609e6c9ac8767";
const V3_EXPECTED_COLUMN_SIGNATURE: &str =
    "7cd1aa5142dbccdc2ac2db466ba4ffdf0c9c41a1000ae0c59baf650e36bbaae8";
const V3_EXPECTED_CONSTRAINT_SIGNATURE: &str =
    "f9d587125d792646b77ca68e6224c9866bd32c87a0e98c4d2f85b75dd0c22be8";
const V3_EXPECTED_INDEX_SIGNATURE: &str =
    "40ca5ea0781b1be03efe9bead50ae9f78434314123d6f700d278874678d06a9b";
const V3_CODEBASE_MEMORY_V2_EXPECTED_RELATION_SIGNATURE: &str =
    "162e7cdb50850fb31348e32ab4516a259fff2543d42fbfe2dd39e4f48679461d";
const V3_CODEBASE_MEMORY_V2_EXPECTED_COLUMN_SIGNATURE: &str =
    "6c71b33bb6ce0adda52c7267a2e15d0f76e80a7da8db847c87155c21db6b574b";
const V3_CODEBASE_MEMORY_V2_EXPECTED_CONSTRAINT_SIGNATURE: &str =
    "272147d02a06e9dd4863efcbc780cd6624d1d74257ae2ef28d8287b6390fe9f7";
const V3_CODEBASE_MEMORY_V2_EXPECTED_INDEX_SIGNATURE: &str =
    "1a7bc5e774689c8ad32c1416dc0cdc6b86a6afca402db2d5eed801c6d71afa5a";
const V4_EXPECTED_RELATION_SIGNATURE: &str =
    "f99237e005c77b7254ae6677d4052eb0397dec0da0333bc61085d5acebfe72ad";
const V4_EXPECTED_COLUMN_SIGNATURE: &str =
    "7376b058f9ce0a3d6564752c16b3e309dd2dfe14f8dde8453359b1c3206efdfe";
const V4_EXPECTED_CONSTRAINT_SIGNATURE: &str =
    "f4decac7be351ebff2b6a74c0a3e0b9cf2ec6ef333ef60e1482984149b807d79";
const V4_EXPECTED_INDEX_SIGNATURE: &str =
    "a1300cfa80ff7b61d41d593f76fece00712c4c16cb39c175e184e149f9926f86";
const EXPECTED_SCHEMA_ACL_SIGNATURE: &str =
    "1bd04ad6cebb5dab6a5a48f47a76e88d19a340bf25aaa49ed9c3270cac479568";
const V3_CODEBASE_MEMORY_V2_EXPECTED_SCHEMA_ACL_SIGNATURE: &str =
    "093efdae2f43f0f5adfdb1296010e990fed1120e54401537939454a2952e7d8e";
const EXPECTED_TABLE_ACL_SIGNATURE: &str =
    "62294e52f6ce6c6c2ab563cc2771b7e4af9f02fb9c891b7592135a7ed7508485";
const EXPECTED_DATABASE_ACL_SIGNATURE: &str =
    "9f805f24430c1a2d452a08969aad3eb8e9c4d4190673c4973ea910595791f425";
const EXPECTED_ROLE_SIGNATURE: &str =
    "cffe9eca3974e4e0fa176553f4dcf2326746b2ed08324a4af51636a02c63a1e0";
const EXPECTED_DEFAULT_ACL_SIGNATURE: &str =
    "25997b75c0fd88d615f4df64a75e0e9180acb5573e0f41db82b3f4735724ba4f";

const RELATION_SIGNATURE_SQL: &str = r"
    SELECT jsonb_build_array(
        n.nspname, c.relname, c.relkind::text, r.rolname,
        c.relpersistence::text, c.relrowsecurity, c.relforcerowsecurity,
        c.relhassubclass, c.relispartition, c.relreplident::text,
        COALESCE(array_to_string(c.reloptions, ','), '<NULL>')
    )::text
    FROM pg_class c
    JOIN pg_namespace n ON n.oid = c.relnamespace
    JOIN pg_roles r ON r.oid = c.relowner
    WHERE n.nspname IN ('control', 'memory', 'readmodel')
      AND c.relkind <> 'i'
    ORDER BY n.nspname, c.relname
";

const COLUMN_SIGNATURE_SQL: &str = r"
    SELECT jsonb_build_array(
        n.nspname, c.relname, a.attnum, a.attname,
        format_type(a.atttypid, a.atttypmod), a.attnotnull, a.attisdropped,
        COALESCE(pg_get_expr(ad.adbin, ad.adrelid, false), '<NULL>'),
        a.attidentity::text, a.attgenerated::text,
        CASE WHEN coll.oid IS NULL THEN '<NULL>'
             ELSE coll_ns.nspname || '.' || coll.collname END,
        a.attstorage::text, a.attcompression::text, a.attstattarget,
        COALESCE(a.attacl::text, '<NULL>')
    )::text
    FROM pg_class c
    JOIN pg_namespace n ON n.oid = c.relnamespace
    JOIN pg_attribute a ON a.attrelid = c.oid
    LEFT JOIN pg_attrdef ad ON ad.adrelid = c.oid AND ad.adnum = a.attnum
    LEFT JOIN pg_collation coll ON coll.oid = a.attcollation
    LEFT JOIN pg_namespace coll_ns ON coll_ns.oid = coll.collnamespace
    WHERE n.nspname IN ('control', 'memory', 'readmodel')
      AND c.relkind IN ('r', 'p')
      AND a.attnum > 0
    ORDER BY n.nspname, c.relname, a.attnum
";

const CONSTRAINT_SIGNATURE_SQL: &str = r"
    SELECT jsonb_build_array(
        n.nspname, c.relname, con.conname, con.contype::text,
        con.convalidated, con.condeferrable, con.condeferred,
        con.connoinherit, con.conislocal, con.coninhcount,
        con.conkey, ref_ns.nspname, ref_class.relname, con.confkey,
        con.confupdtype::text, con.confdeltype::text, con.confmatchtype::text,
        pg_get_constraintdef(con.oid, false)
    )::text
    FROM pg_constraint con
    JOIN pg_namespace n ON n.oid = con.connamespace
    JOIN pg_class c ON c.oid = con.conrelid
    LEFT JOIN pg_class ref_class ON ref_class.oid = con.confrelid
    LEFT JOIN pg_namespace ref_ns ON ref_ns.oid = ref_class.relnamespace
    WHERE n.nspname IN ('control', 'memory', 'readmodel')
    ORDER BY n.nspname, c.relname, con.conname
";

const INDEX_SIGNATURE_SQL: &str = r"
    SELECT jsonb_build_array(
        n.nspname, table_class.relname, index_class.relname,
        i.indisunique, i.indisprimary, i.indisvalid, i.indisready,
        i.indislive, i.indisclustered, i.indisreplident,
        i.indnullsnotdistinct, pg_get_indexdef(i.indexrelid, 0, true)
    )::text
    FROM pg_index i
    JOIN pg_class table_class ON table_class.oid = i.indrelid
    JOIN pg_class index_class ON index_class.oid = i.indexrelid
    JOIN pg_namespace n ON n.oid = table_class.relnamespace
    WHERE n.nspname IN ('control', 'memory', 'readmodel')
    ORDER BY n.nspname, table_class.relname, index_class.relname
";

const SCHEMA_ACL_SIGNATURE_SQL: &str = r"
    SELECT jsonb_build_array(
        n.nspname, owner.rolname, COALESCE(grantee.rolname, 'PUBLIC'),
        grantor.rolname, acl.privilege_type, acl.is_grantable
    )::text
    FROM pg_namespace n
    JOIN pg_roles owner ON owner.oid = n.nspowner
    CROSS JOIN LATERAL aclexplode(COALESCE(n.nspacl, acldefault('n', n.nspowner))) acl
    LEFT JOIN pg_roles grantee ON grantee.oid = acl.grantee
    JOIN pg_roles grantor ON grantor.oid = acl.grantor
    WHERE n.nspname IN ('control', 'memory', 'readmodel')
    ORDER BY n.nspname, owner.rolname, COALESCE(grantee.rolname, 'PUBLIC'),
             grantor.rolname, acl.privilege_type, acl.is_grantable
";

const TABLE_ACL_SIGNATURE_SQL: &str = r"
    SELECT jsonb_build_array(
        n.nspname, c.relname, owner.rolname, COALESCE(grantee.rolname, 'PUBLIC'),
        grantor.rolname, acl.privilege_type, acl.is_grantable
    )::text
    FROM pg_class c
    JOIN pg_namespace n ON n.oid = c.relnamespace
    JOIN pg_roles owner ON owner.oid = c.relowner
    CROSS JOIN LATERAL aclexplode(COALESCE(c.relacl, acldefault('r', c.relowner))) acl
    LEFT JOIN pg_roles grantee ON grantee.oid = acl.grantee
    JOIN pg_roles grantor ON grantor.oid = acl.grantor
    WHERE n.nspname IN ('control', 'memory', 'readmodel')
      AND c.relkind IN ('r', 'p')
    ORDER BY n.nspname, c.relname, owner.rolname,
             COALESCE(grantee.rolname, 'PUBLIC'), grantor.rolname,
             acl.privilege_type, acl.is_grantable
";

const TYPE_SIGNATURE_SQL: &str = r"
    SELECT n.nspname, t.typname, t.typtype::text, t.typisdefined,
           owner.rolname, COALESCE(c.relname, '<NULL>'),
           COALESCE(element_ns.nspname || '.' || element.typname, '<NULL>'),
           COALESCE(array_ns.nspname || '.' || array_type.typname, '<NULL>')
    FROM pg_type t
    JOIN pg_namespace n ON n.oid = t.typnamespace
    JOIN pg_roles owner ON owner.oid = t.typowner
    LEFT JOIN pg_class c ON c.oid = t.typrelid
    LEFT JOIN pg_type element ON element.oid = t.typelem
    LEFT JOIN pg_namespace element_ns ON element_ns.oid = element.typnamespace
    LEFT JOIN pg_type array_type ON array_type.oid = t.typarray
    LEFT JOIN pg_namespace array_ns ON array_ns.oid = array_type.typnamespace
    WHERE n.nspname IN ('control', 'memory', 'readmodel')
    ORDER BY n.nspname, t.typname
";

const FUNCTION_SIGNATURE_SQL: &str = r"
    SELECT jsonb_build_array(
        n.nspname, p.proname,
        pg_get_function_identity_arguments(p.oid),
        pg_get_function_result(p.oid), owner.rolname, language.lanname,
        p.prokind::text, p.prosecdef, p.proleakproof,
        p.provolatile::text, p.proparallel::text, p.proisstrict, p.proretset,
        p.pronargs, p.pronargdefaults, p.prorettype::regtype::text,
        p.proargtypes::text, COALESCE(p.proallargtypes::text, '<NULL>'),
        COALESCE(p.proargmodes::text, '<NULL>'),
        COALESCE(p.proargnames::text, '<NULL>'),
        COALESCE(array_to_string(p.proconfig, ','), '<NULL>'),
        COALESCE(p.probin, '<NULL>'), p.prosrc,
        pg_get_functiondef(p.oid)
    )::text
    FROM pg_proc p
    JOIN pg_namespace n ON n.oid = p.pronamespace
    JOIN pg_roles owner ON owner.oid = p.proowner
    JOIN pg_language language ON language.oid = p.prolang
    WHERE n.nspname IN ('control', 'memory', 'readmodel')
    ORDER BY n.nspname, p.proname, pg_get_function_identity_arguments(p.oid)
";
const V2_EXPECTED_FUNCTION_SIGNATURE: &str =
    "1e0a53bff3e47d1accf1da1e0856b8edf77738fb5a20f9163b9d2d5747481064";
const V3_EXPECTED_FUNCTION_SIGNATURE: &str =
    "f2c8585e1da944b38a50c65c6b9f448963f4c3d96c909331be87fec0c30d2279";
const V3_CODEBASE_MEMORY_V2_EXPECTED_FUNCTION_SIGNATURE: &str =
    "52e146d4e8190bf92ada1754f233423055a435cf281975f05fc83b262ff20db6";
const V4_EXPECTED_FUNCTION_SIGNATURE: &str =
    "557102df8882970df2c71a96b08998ee6d4c6a12d8cf312118ad80d8e1ad1c75";
const FUNCTION_ACL_SIGNATURE_SQL: &str = r"
    SELECT jsonb_build_array(
        n.nspname, p.proname, pg_get_function_identity_arguments(p.oid),
        COALESCE(grantee.rolname, 'PUBLIC'), grantor.rolname,
        acl.privilege_type, acl.is_grantable
    )::text
    FROM pg_proc p
    JOIN pg_namespace n ON n.oid = p.pronamespace
    CROSS JOIN LATERAL aclexplode(COALESCE(p.proacl, acldefault('f', p.proowner))) acl
    LEFT JOIN pg_roles grantee ON grantee.oid = acl.grantee
    JOIN pg_roles grantor ON grantor.oid = acl.grantor
    WHERE n.nspname IN ('control', 'memory', 'readmodel')
    ORDER BY n.nspname, p.proname, pg_get_function_identity_arguments(p.oid),
             COALESCE(grantee.rolname, 'PUBLIC'), grantor.rolname,
             acl.privilege_type, acl.is_grantable
";
const V2_EXPECTED_FUNCTION_ACL_SIGNATURE: &str =
    "ceedd8bee4f0f430505385bac5cee89e1fdb1ce42a72965fd1b1b4ecad0ac5e5";
const V3_EXPECTED_FUNCTION_ACL_SIGNATURE: &str =
    "579b843df8e187eb0f4b7a75e9d1b0c4f109d596c55bcff5aa76a1a06bfcd91b";
const V3_CODEBASE_MEMORY_V2_EXPECTED_FUNCTION_ACL_SIGNATURE: &str =
    "009fc8df8b1ec0867fdfdecf464d24ad694c61d639ff264f56ffb535a2f3038a";
const V4_EXPECTED_FUNCTION_ACL_SIGNATURE: &str =
    "d4556d81218a40d600a72c53096b939afc45efc8ab537baca556c69a3ea11a0f";
const V3_EXPECTED_TABLE_ACL_SIGNATURE: &str =
    "27a0879d1b709abd341653b445d3a64d59819bde2e20e868ac09d2624aab1993";
const V3_CODEBASE_MEMORY_V2_EXPECTED_TABLE_ACL_SIGNATURE: &str =
    "273197d8086b87d4e3308afcc19e34d4b558c0723a23f6965fb07c8ad46f5770";
const V4_EXPECTED_TABLE_ACL_SIGNATURE: &str =
    "641f261e2cc1c93786eda9ac80fbcdb497e719708ad569bee65e9d451b43d2b0";

const DATABASE_ACL_SIGNATURE_SQL: &str = r"
    SELECT jsonb_build_array(
        owner.rolname, COALESCE(grantee.rolname, 'PUBLIC'), grantor.rolname,
        acl.privilege_type, acl.is_grantable
    )::text
    FROM pg_database d
    JOIN pg_roles owner ON owner.oid = d.datdba
    CROSS JOIN LATERAL aclexplode(COALESCE(d.datacl, acldefault('d', d.datdba))) acl
    LEFT JOIN pg_roles grantee ON grantee.oid = acl.grantee
    JOIN pg_roles grantor ON grantor.oid = acl.grantor
    WHERE d.datname = current_database()
    ORDER BY owner.rolname, COALESCE(grantee.rolname, 'PUBLIC'), grantor.rolname,
             acl.privilege_type, acl.is_grantable
";

const ROLE_SIGNATURE_SQL: &str = r"
    SELECT jsonb_build_array(
        rolname, rolsuper, rolinherit, rolcreaterole, rolcreatedb,
        rolcanlogin, rolreplication, rolbypassrls, rolconnlimit,
        COALESCE(rolvaliduntil::text, '<NULL>')
    )::text
    FROM pg_roles
    WHERE rolname IN ('lattice_migrator', 'lattice_runtime',
                      'lattice_guardian', 'lattice_readonly',
                      'lattice_migrator_login', 'lattice_runtime_login',
                      'lattice_guardian_login', 'lattice_readonly_login')
    ORDER BY rolname
";

const DEFAULT_ACL_SIGNATURE_SQL: &str = r"
    SELECT jsonb_build_array(
        CASE WHEN d.defaclnamespace = 0 THEN '<GLOBAL>' ELSE n.nspname END,
        d.defaclobjtype::text, COALESCE(grantee.rolname, 'PUBLIC'), grantor.rolname,
        acl.privilege_type, acl.is_grantable
    )::text
    FROM pg_default_acl d
    LEFT JOIN pg_namespace n ON n.oid = d.defaclnamespace
    CROSS JOIN LATERAL aclexplode(d.defaclacl) acl
    LEFT JOIN pg_roles grantee ON grantee.oid = acl.grantee
    JOIN pg_roles grantor ON grantor.oid = acl.grantor
    WHERE d.defaclrole = 'lattice_migrator'::regrole
    ORDER BY CASE WHEN d.defaclnamespace = 0 THEN '<GLOBAL>' ELSE n.nspname END,
             d.defaclobjtype::text, COALESCE(grantee.rolname, 'PUBLIC'),
             grantor.rolname, acl.privilege_type, acl.is_grantable
";
const CONTROL_SCHEMAS: [&str; 3] = ["control", "memory", "readmodel"];
const CONTROL_TABLES: [&str; 6] = [
    "database_identity",
    "migration_history",
    "physical_heads",
    "runtime_admission",
    "schema_compatibility",
    "terminal_transactions",
];
const V3_CONTROL_TABLES: [&str; 10] = [
    "database_identity",
    "migration_history",
    "physical_heads",
    "runtime_admission",
    "schema_compatibility",
    "task_ledger_commands",
    "task_ledger_events",
    "task_ledger_outbox",
    "task_ledger_streams",
    "terminal_transactions",
];
const V4_CONTROL_TABLES: [&str; 15] = [
    "database_identity",
    "migration_history",
    "physical_heads",
    "project_registry_commands",
    "project_registry_identity_reservations",
    "project_registry_observations",
    "project_registry_projects",
    "project_registry_state",
    "runtime_admission",
    "schema_compatibility",
    "task_ledger_commands",
    "task_ledger_events",
    "task_ledger_outbox",
    "task_ledger_streams",
    "terminal_transactions",
];
const READABLE_CONTROL_TABLES: [&str; 4] = [
    "database_identity",
    "migration_history",
    "runtime_admission",
    "schema_compatibility",
];
const PROTECTED_CONTROL_TABLES: [&str; 2] = ["physical_heads", "terminal_transactions"];
const V3_PROTECTED_CONTROL_TABLES: [&str; 6] = [
    "physical_heads",
    "task_ledger_commands",
    "task_ledger_events",
    "task_ledger_outbox",
    "task_ledger_streams",
    "terminal_transactions",
];
const V4_PROTECTED_CONTROL_TABLES: [&str; 11] = [
    "physical_heads",
    "project_registry_commands",
    "project_registry_identity_reservations",
    "project_registry_observations",
    "project_registry_projects",
    "project_registry_state",
    "task_ledger_commands",
    "task_ledger_events",
    "task_ledger_outbox",
    "task_ledger_streams",
    "terminal_transactions",
];
const CODEBASE_MEMORY_V2_TABLES: [&str; 8] = [
    "codebase_memory_analyses",
    "codebase_memory_extension_identity",
    "codebase_memory_extension_ledger",
    "codebase_memory_receipts",
    "codebase_memory_records",
    "codebase_memory_reflections",
    "codebase_memory_retrieval_audits",
    "openclaw_gateway_commands",
];
const CODEBASE_MEMORY_V2_FUNCTIONS: [&str; 7] = [
    "codebase_memory_load_reflection_v2",
    "codebase_memory_load_receipt_v1",
    "codebase_memory_persist_analysis_v1",
    "codebase_memory_persist_reflection_v2",
    "codebase_memory_persist_retrieval_v1",
    "openclaw_gateway_finalize_terminal_v1",
    "openclaw_gateway_reconcile_and_claim_v1",
];
const WRITER_LEASE_V1_TABLES: [&str; 5] = [
    "writer_lease_commands",
    "writer_lease_extension_identity",
    "writer_lease_extension_ledger",
    "writer_lease_heads",
    "writer_lease_transitions",
];
const WRITER_LEASE_V1_FUNCTIONS: [&str; 7] = [
    "writer_lease_assert_current_v1",
    "writer_lease_bind_runtime_v1",
    "writer_lease_commit_plan_v1",
    "writer_lease_load_commands_v1",
    "writer_lease_load_current_v1",
    "writer_lease_load_for_update_v1",
    "writer_lease_load_transitions_v1",
];
const WRITER_LEASE_V1_RELATION_PROFILE_SQL: &str = "SELECT pg_catalog.jsonb_build_array(\
    n.nspname,c.relname,c.relkind::text,o.rolname,c.relpersistence::text,c.relrowsecurity,\
    c.relforcerowsecurity,c.relhassubclass,c.relispartition,c.relreplident::text,\
    COALESCE(pg_catalog.array_to_string(c.reloptions,','),'<NULL>'),\
    COALESCE(pg_catalog.obj_description(c.oid,'pg_class'),'<NULL>'))::text \
    FROM pg_catalog.pg_class c \
    JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace \
    JOIN pg_catalog.pg_roles o ON o.oid=c.relowner \
    WHERE n.nspname='writer_lease' AND c.relkind IN ('r','p') \
    ORDER BY n.nspname,c.relname";
const WRITER_LEASE_V1_COLUMN_PROFILE_SQL: &str = "SELECT pg_catalog.jsonb_build_array(\
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
const WRITER_LEASE_V1_CONSTRAINT_PROFILE_SQL: &str = "SELECT pg_catalog.jsonb_build_array(\
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
const WRITER_LEASE_V1_INDEX_PROFILE_SQL: &str = "SELECT pg_catalog.jsonb_build_array(\
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
const WRITER_LEASE_V1_FUNCTION_PROFILE_SQL: &str = "SELECT pg_catalog.jsonb_build_array(\
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
const WRITER_LEASE_V1_SCHEMA_ACL_PROFILE_SQL: &str = "SELECT pg_catalog.jsonb_build_array(\
    n.nspname,o.rolname,COALESCE(g.rolname,'PUBLIC'),r.rolname,a.privilege_type,a.is_grantable)::text \
    FROM pg_catalog.pg_namespace n JOIN pg_catalog.pg_roles o ON o.oid=n.nspowner \
    CROSS JOIN LATERAL pg_catalog.aclexplode(COALESCE(n.nspacl,pg_catalog.acldefault('n',n.nspowner))) a \
    LEFT JOIN pg_catalog.pg_roles g ON g.oid=a.grantee JOIN pg_catalog.pg_roles r ON r.oid=a.grantor \
    WHERE n.nspname='writer_lease' ORDER BY n.nspname,o.rolname,COALESCE(g.rolname,'PUBLIC'),\
    r.rolname,a.privilege_type,a.is_grantable";
const WRITER_LEASE_V1_TABLE_ACL_PROFILE_SQL: &str = "SELECT pg_catalog.jsonb_build_array(\
    n.nspname,c.relname,o.rolname,COALESCE(g.rolname,'PUBLIC'),r.rolname,a.privilege_type,\
    a.is_grantable)::text FROM pg_catalog.pg_class c \
    JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace \
    JOIN pg_catalog.pg_roles o ON o.oid=c.relowner \
    CROSS JOIN LATERAL pg_catalog.aclexplode(COALESCE(c.relacl,pg_catalog.acldefault('r',c.relowner))) a \
    LEFT JOIN pg_catalog.pg_roles g ON g.oid=a.grantee JOIN pg_catalog.pg_roles r ON r.oid=a.grantor \
    WHERE n.nspname='writer_lease' AND c.relkind IN ('r','p') \
    ORDER BY n.nspname,c.relname,o.rolname,COALESCE(g.rolname,'PUBLIC'),r.rolname,\
    a.privilege_type,a.is_grantable";
const WRITER_LEASE_V1_FUNCTION_ACL_PROFILE_SQL: &str = "SELECT pg_catalog.jsonb_build_array(\
    n.nspname,p.proname,pg_catalog.pg_get_function_identity_arguments(p.oid),o.rolname,\
    COALESCE(g.rolname,'PUBLIC'),r.rolname,a.privilege_type,a.is_grantable)::text \
    FROM pg_catalog.pg_proc p JOIN pg_catalog.pg_namespace n ON n.oid=p.pronamespace \
    JOIN pg_catalog.pg_roles o ON o.oid=p.proowner \
    CROSS JOIN LATERAL pg_catalog.aclexplode(COALESCE(p.proacl,pg_catalog.acldefault('f',p.proowner))) a \
    LEFT JOIN pg_catalog.pg_roles g ON g.oid=a.grantee JOIN pg_catalog.pg_roles r ON r.oid=a.grantor \
    WHERE n.nspname='writer_lease' ORDER BY n.nspname,p.proname,\
    pg_catalog.pg_get_function_identity_arguments(p.oid),o.rolname,COALESCE(g.rolname,'PUBLIC'),\
    r.rolname,a.privilege_type,a.is_grantable";
const WRITER_LEASE_V1_COLUMN_ACL_PROFILE_SQL: &str = "SELECT pg_catalog.jsonb_build_array(\
    n.nspname,c.relname,a.attnum,a.attname,COALESCE(g.rolname,'PUBLIC'),r.rolname,x.privilege_type,\
    x.is_grantable)::text FROM pg_catalog.pg_class c \
    JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace \
    JOIN pg_catalog.pg_attribute a ON a.attrelid=c.oid \
    CROSS JOIN LATERAL pg_catalog.aclexplode(a.attacl) x \
    LEFT JOIN pg_catalog.pg_roles g ON g.oid=x.grantee JOIN pg_catalog.pg_roles r ON r.oid=x.grantor \
    WHERE n.nspname='writer_lease' AND c.relkind IN ('r','p') AND a.attnum>0 \
    ORDER BY n.nspname,c.relname,a.attnum,COALESCE(g.rolname,'PUBLIC'),r.rolname,\
    x.privilege_type,x.is_grantable";
const WRITER_LEASE_V1_TYPE_PROFILE_SQL: &str = "SELECT pg_catalog.jsonb_build_array(\
    n.nspname,t.typname,t.typtype::text,t.typcategory::text,t.typispreferred,t.typisdefined,\
    t.typdelim::text,o.rolname,COALESCE(c.relname,'<NULL>'),COALESCE(e.typname,'<NULL>'),\
    COALESCE(pg_catalog.obj_description(t.oid,'pg_type'),'<NULL>'))::text \
    FROM pg_catalog.pg_type t JOIN pg_catalog.pg_namespace n ON n.oid=t.typnamespace \
    JOIN pg_catalog.pg_roles o ON o.oid=t.typowner \
    LEFT JOIN pg_catalog.pg_class c ON c.oid=t.typrelid \
    LEFT JOIN pg_catalog.pg_type e ON e.oid=t.typelem \
    WHERE n.nspname='writer_lease' ORDER BY n.nspname,t.typname";
const WRITER_LEASE_V1_CATALOG_PROFILES: [(&str, &str, PostgresStoreSetupErrorKind); 10] = [
    (
        WRITER_LEASE_V1_RELATION_PROFILE_SQL,
        "c20048700ff120bc6488c4608eb79df36d329ad817f2b0e45e47020d867b8251",
        PostgresStoreSetupErrorKind::CorruptCatalog,
    ),
    (
        WRITER_LEASE_V1_COLUMN_PROFILE_SQL,
        "560e93c2a765db0024c0e74d25a51b90cfc72b204601139de8fdb688d48c0610",
        PostgresStoreSetupErrorKind::CorruptCatalog,
    ),
    (
        WRITER_LEASE_V1_CONSTRAINT_PROFILE_SQL,
        "3deab2f6ee712692d5ec75682030462ebd4dd4712ff26e40bf323abdd683c5d3",
        PostgresStoreSetupErrorKind::CorruptCatalog,
    ),
    (
        WRITER_LEASE_V1_INDEX_PROFILE_SQL,
        "a30a0abfca0a824d75f2f29eb85a8424af35da485f1fef1bc5f852b9be7151a4",
        PostgresStoreSetupErrorKind::CorruptCatalog,
    ),
    (
        WRITER_LEASE_V1_FUNCTION_PROFILE_SQL,
        "638941fbd31edbec9d9f860974aac280845063693acc00bd5f6f8c3aa650adc9",
        PostgresStoreSetupErrorKind::CorruptCatalog,
    ),
    (
        WRITER_LEASE_V1_SCHEMA_ACL_PROFILE_SQL,
        "a2e1be8a403a96b679c18ddfa75e476fa1d6ceeccc1ccf62ff6424b2c259ef7b",
        PostgresStoreSetupErrorKind::PermissionDenied,
    ),
    (
        WRITER_LEASE_V1_TABLE_ACL_PROFILE_SQL,
        "b99ef0c0ea5b550ae5e805d29b0020e31c1800a016b0de82cda566d7b25e9569",
        PostgresStoreSetupErrorKind::PermissionDenied,
    ),
    (
        WRITER_LEASE_V1_FUNCTION_ACL_PROFILE_SQL,
        "4e1a2ba0c5abcfe928b66b839166f2bebeecca73a0514f02344c9bbb695b0c44",
        PostgresStoreSetupErrorKind::PermissionDenied,
    ),
    (
        WRITER_LEASE_V1_COLUMN_ACL_PROFILE_SQL,
        "a7ccfc938fbf121a9b807070f69bd5b851be6aa89a8261043ef07336ea7b8dbd",
        PostgresStoreSetupErrorKind::PermissionDenied,
    ),
    (
        WRITER_LEASE_V1_TYPE_PROFILE_SQL,
        "1d6642e77600a93da5b00dda0ee64c15474b4ca2741c51ca760597e7f90ac003",
        PostgresStoreSetupErrorKind::CorruptCatalog,
    ),
];
const STORE_PREPARE_V2_IDENTITY: &str = "control.store_prepare_v2(smallint,text,text,text,text,bytea,bytea,text,text,bigint,text,bigint,bytea,bytea,text,bigint,bytea,bytea,bytea,bytea,bytea,bytea,bytea,bytea,bytea,bytea)";
const STORE_FINALIZE_V2_IDENTITY: &str = "control.store_finalize_v2(smallint,text,text,text,text,bytea,bytea,text,text,bigint,text,bigint,bytea,bytea,text,bigint,bytea,bytea,bytea,bytea,bytea,bytea,bytea,bytea,bytea,bytea,uuid,bytea,smallint,text,bigint,bytea,bytea,bigint,bytea,bytea,text,bytea,bytea)";
const STORE_CURRENT_HEAD_V2_IDENTITY: &str = "control.store_current_head_v2(text,text,text,bytea)";
const STORE_PREPARE_V3_IDENTITY: &str = "control.store_prepare_v3(smallint,text,text,text,text,bytea,bytea,text,text,bigint,text,bigint,bytea,bytea,text,bigint,bytea,bytea,bytea,bytea,bytea,bytea,bytea,bytea,bytea,bytea)";
const STORE_FINALIZE_V3_IDENTITY: &str = "control.store_finalize_v3(smallint,text,text,text,text,bytea,bytea,text,text,bigint,text,bigint,bytea,bytea,text,bigint,bytea,bytea,bytea,bytea,bytea,bytea,bytea,bytea,bytea,bytea,uuid,bytea,smallint,text,bigint,bytea,bytea,bigint,bytea,bytea,text,bytea,bytea)";
const STORE_CURRENT_HEAD_V3_IDENTITY: &str = "control.store_current_head_v3(text,text,text,bytea)";
const TASK_LEDGER_PREPARE_V1_IDENTITY: &str = "control.task_ledger_prepare_v1(bytea,text)";
const TASK_LEDGER_READ_HEAD_V1_IDENTITY: &str = "control.task_ledger_read_head_v1(bytea,text,text)";
const TASK_LEDGER_READ_EVENTS_V1_IDENTITY: &str = "control.task_ledger_read_events_v1(bytea)";
const TASK_LEDGER_READ_COMMANDS_V1_IDENTITY: &str = "control.task_ledger_read_commands_v1(bytea)";
const TASK_LEDGER_FINALIZE_V1_IDENTITY: &str = "control.task_ledger_finalize_v1(bytea,text,text,text,text,bytea,text,text,bytea,text,bytea,bytea,text,text,text,text,text,text,text,text,text,bytea,bytea,text,bytea,text,bytea,text,bytea,bytea,text,text,text,text,text,text,text,bytea,jsonb,boolean,text,text,text,text,text,text,text,bytea,text,bytea,bytea,text,bytea,text,bytea,bytea,text,text,bytea,bytea,bytea,text,boolean,text,bytea,text,bytea,boolean,bytea,bytea)";
const STORE_PREPARE_V4_IDENTITY: &str = "control.store_prepare_v4(smallint,text,smallint,text,text,text,text,bytea,bytea,text,text,bigint,text,bigint,bytea,bytea,text,bigint,bytea,bytea,bytea,bytea,bytea,bytea,bytea,bytea,bytea,bytea)";
const STORE_FINALIZE_V4_IDENTITY: &str = "control.store_finalize_v4(smallint,text,smallint,text,text,text,text,bytea,bytea,text,text,bigint,text,bigint,bytea,bytea,text,bigint,bytea,bytea,bytea,bytea,bytea,bytea,bytea,bytea,bytea,bytea,uuid,bytea,smallint,text,bigint,bytea,bytea,bigint,bytea,bytea,text,bytea,bytea)";
const STORE_CURRENT_HEAD_V4_IDENTITY: &str =
    "control.store_current_head_v4(smallint,text,text,text,text,bytea)";
const TASK_LEDGER_PREPARE_V2_IDENTITY: &str =
    "control.task_ledger_prepare_v2(smallint,text,bytea,text)";
const TASK_LEDGER_READ_HEAD_V2_IDENTITY: &str =
    "control.task_ledger_read_head_v2(smallint,text,bytea,text,text)";
const TASK_LEDGER_READ_EVENTS_V2_IDENTITY: &str =
    "control.task_ledger_read_events_v2(smallint,text,bytea)";
const TASK_LEDGER_READ_COMMANDS_V2_IDENTITY: &str =
    "control.task_ledger_read_commands_v2(smallint,text,bytea)";
const TASK_LEDGER_FINALIZE_V2_IDENTITY: &str = "control.task_ledger_finalize_v2(smallint,text,bytea,text,text,text,text,bytea,text,text,bytea,text,bytea,bytea,text,text,text,text,text,text,text,text,text,bytea,bytea,text,bytea,text,bytea,text,bytea,bytea,text,text,text,text,text,text,text,bytea,jsonb,boolean,text,text,text,text,text,text,text,bytea,text,bytea,bytea,text,bytea,text,bytea,bytea,text,text,bytea,bytea,bytea,text,boolean,text,bytea,text,bytea,boolean,bytea,bytea)";
const PROJECT_REGISTRY_PREPARE_V1_IDENTITY: &str = "control.project_registry_prepare_v1(smallint,text,text,bytea,text,text,bigint,text,bigint,bytea,bytea,bytea)";
const PROJECT_REGISTRY_READ_STATE_V1_IDENTITY: &str =
    "control.project_registry_read_state_v1(smallint,text)";
const PROJECT_REGISTRY_READ_OBSERVATIONS_V1_IDENTITY: &str =
    "control.project_registry_read_observations_v1(smallint,text)";
const PROJECT_REGISTRY_READ_PROJECTS_V1_IDENTITY: &str =
    "control.project_registry_read_projects_v1(smallint,text)";
const PROJECT_REGISTRY_READ_COMMANDS_V1_IDENTITY: &str =
    "control.project_registry_read_commands_v1(smallint,text)";
const PROJECT_REGISTRY_READ_RESERVATIONS_V1_IDENTITY: &str =
    "control.project_registry_read_reservations_v1(smallint,text)";
const PROJECT_REGISTRY_STAGE_COMMAND_V1_IDENTITY: &str = "control.project_registry_stage_command_v1(smallint,text,bigint,text,text,text,text,bytea,boolean,text,text,text,text,text,numeric,text,text,text,bytea,bytea,bytea,text,bytea,bytea,text,text,text,text,text,text,text,bytea,bytea,bytea,boolean,boolean,boolean,boolean,boolean,bytea,text,bigint,bigint,bigint,bigint,bigint,bigint,bytea,text,bigint,bigint,bigint,bigint,bigint,bigint,bytea,bytea,text,text,bigint,text,bigint,bytea,bytea,bytea,bytea,boolean,text,bytea,bytea,bytea,text,bytea)";
const PROJECT_REGISTRY_STAGE_PROJECT_V1_IDENTITY: &str = "control.project_registry_stage_project_v1(smallint,text,text,text,bytea,bytea,boolean,boolean,boolean,boolean,boolean,smallint,text,text,text,text,numeric,text,text,bytea,bytea,bytea)";
const PROJECT_REGISTRY_FINALIZE_V1_IDENTITY: &str = "control.project_registry_finalize_v1(smallint,text,text,bigint,text,bigint,bigint,bigint,bigint,bigint,bigint,bytea,text,bigint,bigint,bigint,bigint,bigint,bigint,bytea,bytea,bytea,bytea,boolean,boolean,bigint,bigint)";
const V4_RUNTIME_FUNCTION_IDENTITIES: [&str; 17] = [
    STORE_PREPARE_V4_IDENTITY,
    STORE_FINALIZE_V4_IDENTITY,
    STORE_CURRENT_HEAD_V4_IDENTITY,
    TASK_LEDGER_PREPARE_V2_IDENTITY,
    TASK_LEDGER_READ_HEAD_V2_IDENTITY,
    TASK_LEDGER_READ_EVENTS_V2_IDENTITY,
    TASK_LEDGER_READ_COMMANDS_V2_IDENTITY,
    TASK_LEDGER_FINALIZE_V2_IDENTITY,
    PROJECT_REGISTRY_PREPARE_V1_IDENTITY,
    PROJECT_REGISTRY_READ_STATE_V1_IDENTITY,
    PROJECT_REGISTRY_READ_OBSERVATIONS_V1_IDENTITY,
    PROJECT_REGISTRY_READ_PROJECTS_V1_IDENTITY,
    PROJECT_REGISTRY_READ_COMMANDS_V1_IDENTITY,
    PROJECT_REGISTRY_READ_RESERVATIONS_V1_IDENTITY,
    PROJECT_REGISTRY_STAGE_COMMAND_V1_IDENTITY,
    PROJECT_REGISTRY_STAGE_PROJECT_V1_IDENTITY,
    PROJECT_REGISTRY_FINALIZE_V1_IDENTITY,
];
const V3_CONTROL_FUNCTION_IDENTITIES: [&str; 11] = [
    STORE_PREPARE_V2_IDENTITY,
    STORE_FINALIZE_V2_IDENTITY,
    STORE_CURRENT_HEAD_V2_IDENTITY,
    STORE_PREPARE_V3_IDENTITY,
    STORE_FINALIZE_V3_IDENTITY,
    STORE_CURRENT_HEAD_V3_IDENTITY,
    TASK_LEDGER_PREPARE_V1_IDENTITY,
    TASK_LEDGER_READ_HEAD_V1_IDENTITY,
    TASK_LEDGER_READ_EVENTS_V1_IDENTITY,
    TASK_LEDGER_READ_COMMANDS_V1_IDENTITY,
    TASK_LEDGER_FINALIZE_V1_IDENTITY,
];
const CODEBASE_MEMORY_LOAD_RECEIPT_V1_IDENTITY: &str = "memory.codebase_memory_load_receipt_v1(bytea,bytea,bytea,bytea,smallint,text,text,text,text,bytea,text,text,bytea,bytea,smallint)";
const CODEBASE_MEMORY_LOAD_REFLECTION_V2_IDENTITY: &str = "memory.codebase_memory_load_reflection_v2(bytea,bytea,bytea,bytea,smallint,text,text,text,text,bytea,text,text,bytea,bytea,smallint)";
const CODEBASE_MEMORY_PERSIST_ANALYSIS_V1_IDENTITY: &str = "memory.codebase_memory_persist_analysis_v1(bytea,bytea,bytea,bytea,smallint,text,text,text,text,bytea,text,text,bytea,bytea,smallint,text,bytea,bytea,bytea,bytea,bytea,bytea,bytea,bytea,bytea,integer[],bytea[],text[],text[],text[],text[],text[],text[],bytea[],integer[],integer[],text[],bytea[])";
const CODEBASE_MEMORY_PERSIST_REFLECTION_V2_IDENTITY: &str = "memory.codebase_memory_persist_reflection_v2(bytea,bytea,bytea,bytea,smallint,text,text,text,text,bytea,text,text,bytea,bytea,smallint,bytea,text,text,bytea,bytea,bytea,bytea,text,text[],bytea[],text[])";
const CODEBASE_MEMORY_PERSIST_RETRIEVAL_V1_IDENTITY: &str = "memory.codebase_memory_persist_retrieval_v1(bytea,bytea,bytea,bytea,bytea,bytea,bytea,smallint,text,bytea[],bytea[],bigint[],bytea,bytea,bytea)";
const OPENCLAW_GATEWAY_FINALIZE_TERMINAL_V1_IDENTITY: &str = "memory.openclaw_gateway_finalize_terminal_v1(bytea,bytea,bytea,bytea,text,text,bigint,text,bytea,bytea,bytea,bytea)";
const OPENCLAW_GATEWAY_RECONCILE_AND_CLAIM_V1_IDENTITY: &str = "memory.openclaw_gateway_reconcile_and_claim_v1(bytea,bytea,bytea,bytea,text,text,bigint,text,bytea)";
const V1_CONTROL_CONSTRAINTS: [&str; 48] = [
    "database_identity_pkey",
    "database_identity_singleton_true",
    "database_identity_uuid_v8",
    "migration_history_migration_id_key",
    "migration_history_migration_path_key",
    "migration_history_mode_matches_status",
    "migration_history_pkey",
    "migration_history_positive_bytes",
    "migration_history_positive_ordinal",
    "migration_history_reader_range",
    "migration_history_safe_id",
    "migration_history_safe_path",
    "migration_history_sha256",
    "migration_history_status",
    "migration_history_transaction_mode",
    "migration_history_writer_range",
    "physical_heads_aggregate_digest",
    "physical_heads_head_digest",
    "physical_heads_owner_closed",
    "physical_heads_pkey",
    "physical_heads_project_id",
    "physical_heads_revision",
    "physical_heads_snapshot_id",
    "physical_heads_state_digest",
    "runtime_admission_authority_shape",
    "runtime_admission_mode_closed",
    "runtime_admission_pkey",
    "runtime_admission_singleton_true",
    "schema_compatibility_current_positive",
    "schema_compatibility_pkey",
    "schema_compatibility_reader_range",
    "schema_compatibility_sha256",
    "schema_compatibility_singleton_true",
    "schema_compatibility_writer_range",
    "terminal_transactions_admission_active",
    "terminal_transactions_authority_positive",
    "terminal_transactions_daemon_instance_id",
    "terminal_transactions_digest_shapes",
    "terminal_transactions_disposition",
    "terminal_transactions_owner_closed",
    "terminal_transactions_pkey",
    "terminal_transactions_project_id",
    "terminal_transactions_required_nonzero",
    "terminal_transactions_revision_transition",
    "terminal_transactions_revisions",
    "terminal_transactions_safe_id",
    "terminal_transactions_scope_head_fk",
    "terminal_transactions_snapshot_id",
];
const V2_CONTROL_CONSTRAINTS: [&str; 55] = [
    "database_identity_pkey",
    "database_identity_singleton_true",
    "database_identity_uuid_v8",
    "migration_history_migration_id_key",
    "migration_history_migration_path_key",
    "migration_history_mode_matches_status",
    "migration_history_pkey",
    "migration_history_positive_bytes",
    "migration_history_positive_ordinal",
    "migration_history_reader_range",
    "migration_history_safe_id",
    "migration_history_safe_path",
    "migration_history_sha256",
    "migration_history_status",
    "migration_history_transaction_mode",
    "migration_history_writer_range",
    "physical_heads_aggregate_digest",
    "physical_heads_head_digest",
    "physical_heads_owner_closed",
    "physical_heads_pkey",
    "physical_heads_project_id",
    "physical_heads_revision",
    "physical_heads_snapshot_id",
    "physical_heads_state_digest",
    "runtime_admission_authority_shape",
    "runtime_admission_mode_closed",
    "runtime_admission_pkey",
    "runtime_admission_singleton_true",
    "schema_compatibility_current_positive",
    "schema_compatibility_pkey",
    "schema_compatibility_reader_range",
    "schema_compatibility_sha256",
    "schema_compatibility_singleton_true",
    "schema_compatibility_writer_range",
    "terminal_transactions_admission_active",
    "terminal_transactions_authority_positive",
    "terminal_transactions_daemon_instance_id",
    "terminal_transactions_database_identity_digest",
    "terminal_transactions_database_uuid_v8",
    "terminal_transactions_digest_shapes",
    "terminal_transactions_disposition",
    "terminal_transactions_durability_postgres",
    "terminal_transactions_manifest_sha256",
    "terminal_transactions_owner_closed",
    "terminal_transactions_pkey",
    "terminal_transactions_producer_exact",
    "terminal_transactions_project_id",
    "terminal_transactions_required_nonzero",
    "terminal_transactions_revision_transition",
    "terminal_transactions_revisions",
    "terminal_transactions_runtime_live",
    "terminal_transactions_safe_id",
    "terminal_transactions_schema_v2",
    "terminal_transactions_snapshot_id",
    "terminal_transactions_store_contract_v2",
];
const TASK_LEDGER_CONTROL_CONSTRAINTS: [&str; 47] = [
    "task_ledger_commands_closed_values",
    "task_ledger_commands_diagnostic_bounded",
    "task_ledger_commands_digest_shapes",
    "task_ledger_commands_identifiers",
    "task_ledger_commands_pkey",
    "task_ledger_commands_resource_shape",
    "task_ledger_commands_store_terminal_fk",
    "task_ledger_commands_stream_fk",
    "task_ledger_commands_timestamp_shape",
    "task_ledger_commands_u64_values",
    "task_ledger_commands_versions_exact",
    "task_ledger_events_closed_values",
    "task_ledger_events_command_fk",
    "task_ledger_events_diagnostic_bounded",
    "task_ledger_events_digest_shapes",
    "task_ledger_events_event_digest_key",
    "task_ledger_events_identifiers",
    "task_ledger_events_pkey",
    "task_ledger_events_projection_shape",
    "task_ledger_events_resource_shape",
    "task_ledger_events_resource_u64",
    "task_ledger_events_sequence_u64",
    "task_ledger_events_stream_id_command_id_key",
    "task_ledger_events_timestamp_shape",
    "task_ledger_events_version_exact",
    "task_ledger_outbox_command_fk",
    "task_ledger_outbox_digest_shapes",
    "task_ledger_outbox_event_digest_key",
    "task_ledger_outbox_event_fk",
    "task_ledger_outbox_identifier",
    "task_ledger_outbox_intent_digest_key",
    "task_ledger_outbox_pkey",
    "task_ledger_outbox_sequence_u64",
    "task_ledger_outbox_stream_id_command_id_key",
    "task_ledger_outbox_stream_id_event_sequence_key",
    "task_ledger_outbox_timestamp_shape",
    "task_ledger_outbox_version_state_exact",
    "task_ledger_streams_cost_canonical",
    "task_ledger_streams_counters_u64",
    "task_ledger_streams_digest_shapes",
    "task_ledger_streams_identity_shape",
    "task_ledger_streams_pkey",
    "task_ledger_streams_position_u64",
    "task_ledger_streams_projection_shape",
    "task_ledger_streams_stream_digest",
    "task_ledger_streams_task_revision_u64",
    "task_ledger_streams_versions_exact",
];
const PROJECT_REGISTRY_CONTROL_CONSTRAINTS: [&str; 38] = [
    "project_registry_commands_action",
    "project_registry_commands_before_shape",
    "project_registry_commands_chain",
    "project_registry_commands_command_id_key",
    "project_registry_commands_digest_lengths",
    "project_registry_commands_id",
    "project_registry_commands_observation_digest_fkey",
    "project_registry_commands_ordinal_positive",
    "project_registry_commands_outcome",
    "project_registry_commands_pkey",
    "project_registry_commands_project_id",
    "project_registry_commands_runtime",
    "project_registry_observations_digest",
    "project_registry_observations_identity_digests",
    "project_registry_observations_pkey",
    "project_registry_observations_primary_ref",
    "project_registry_observations_root_bound",
    "project_registry_projects_accepted_observation_digest_fkey",
    "project_registry_projects_authority",
    "project_registry_projects_authority_observation_digest_fkey",
    "project_registry_projects_class",
    "project_registry_projects_id",
    "project_registry_projects_pending_distinct",
    "project_registry_projects_pending_observation_digest_fkey",
    "project_registry_projects_pkey",
    "project_registry_projects_shape",
    "project_registry_reservations_digest",
    "project_registry_reservations_dimension",
    "project_registry_identity_reservations_pkey",
    "project_registry_identity_reservations_project_id_fkey",
    "project_registry_reservations_status",
    "project_registry_state_counts_nonnegative",
    "project_registry_state_digest",
    "project_registry_state_limits",
    "project_registry_state_pkey",
    "project_registry_state_runtime_live",
    "project_registry_state_singleton_true",
    "project_registry_state_stage_shape",
];

/// Result of one explicit administrative migration attempt.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MigrationApplyOutcome {
    /// One or more executable manifest entries were committed.
    Applied { executable_count: usize },
    /// Exact history and catalog were already current.
    AlreadyCurrent,
}

impl MigrationApplyOutcome {
    #[must_use]
    pub const fn executable_count(self) -> usize {
        match self {
            Self::Applied { executable_count } => executable_count,
            Self::AlreadyCurrent => 0,
        }
    }

    #[must_use]
    pub const fn was_current(self) -> bool {
        matches!(self, Self::AlreadyCurrent)
    }
}

/// Only migration-bootstrap admission represented by setup evidence.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum BootstrapAdmission {
    StoppedNoLeader,
}

impl BootstrapAdmission {
    pub const ALL: [Self; 1] = [Self::StoppedNoLeader];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::StoppedNoLeader => "STOPPED_NO_LEADER",
        }
    }
}

/// Read-only exact schema compatibility evidence; not a Store/domain receipt.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PostgresSchemaEvidence {
    database_uuid: String,
    manifest_sha256: Sha256Hex,
    schema_version: u16,
    server_version_num: u32,
    role: DatabaseRole,
    bootstrap_admission: BootstrapAdmission,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RuntimeStoreSchemaEvidence {
    database_uuid: String,
    global_manifest_sha256: Sha256Hex,
    global_schema_version: u16,
    store_manifest_sha256: Sha256Hex,
    store_schema_version: u16,
}

impl RuntimeStoreSchemaEvidence {
    pub(crate) fn database_uuid(&self) -> &str {
        &self.database_uuid
    }

    pub(crate) const fn manifest_sha256(&self) -> &Sha256Hex {
        &self.store_manifest_sha256
    }

    pub(crate) const fn schema_version(&self) -> u16 {
        self.store_schema_version
    }

    pub(crate) const fn global_manifest_sha256(&self) -> &Sha256Hex {
        &self.global_manifest_sha256
    }

    pub(crate) const fn global_schema_version(&self) -> u16 {
        self.global_schema_version
    }
}

impl PostgresSchemaEvidence {
    #[must_use]
    pub fn database_uuid(&self) -> &str {
        &self.database_uuid
    }

    #[must_use]
    pub const fn manifest_sha256(&self) -> &Sha256Hex {
        &self.manifest_sha256
    }

    #[must_use]
    pub const fn schema_version(&self) -> u16 {
        self.schema_version
    }

    #[must_use]
    pub const fn server_version_num(&self) -> u32 {
        self.server_version_num
    }

    #[must_use]
    pub const fn role(&self) -> DatabaseRole {
        self.role
    }

    #[must_use]
    pub const fn bootstrap_admission(&self) -> BootstrapAdmission {
        self.bootstrap_admission
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ConnectionEvidence {
    server_version_num: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SetupOperation {
    Migration,
    Verification,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum InstalledManifestState {
    Fresh,
    ExactV1Prefix,
    ExactV2Prefix,
    ExactV3Prefix,
    ExactV4Full,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CatalogProfile {
    PreSchema,
    V1,
    V2,
    V3,
    V3CodebaseMemoryV2,
    V3CodebaseMemoryV2WriterLeaseV1,
    V4,
}

#[allow(clippy::too_many_arguments)]
fn classify_extension_catalog_counts(
    schema_version: u16,
    expected_memory_relations: i64,
    all_memory_relations: i64,
    expected_memory_functions: i64,
    all_memory_functions: i64,
    writer_lease_namespaces: i64,
    expected_writer_lease_relations: i64,
    all_writer_lease_relations: i64,
    expected_writer_lease_functions: i64,
    all_writer_lease_functions: i64,
) -> Result<CatalogProfile, PostgresStoreSetupError> {
    let no_memory = expected_memory_relations == 0
        && all_memory_relations == 0
        && expected_memory_functions == 0
        && all_memory_functions == 0;
    let exact_memory = expected_memory_relations
        == i64::try_from(CODEBASE_MEMORY_V2_TABLES.len()).expect("fixed count")
        && all_memory_relations == expected_memory_relations
        && expected_memory_functions
            == i64::try_from(CODEBASE_MEMORY_V2_FUNCTIONS.len()).expect("fixed count")
        && all_memory_functions == expected_memory_functions;
    let no_writer_lease = writer_lease_namespaces == 0
        && expected_writer_lease_relations == 0
        && all_writer_lease_relations == 0
        && expected_writer_lease_functions == 0
        && all_writer_lease_functions == 0;
    let exact_writer_lease = writer_lease_namespaces == 1
        && expected_writer_lease_relations
            == i64::try_from(WRITER_LEASE_V1_TABLES.len()).expect("fixed count")
        && all_writer_lease_relations == expected_writer_lease_relations
        && expected_writer_lease_functions
            == i64::try_from(WRITER_LEASE_V1_FUNCTIONS.len()).expect("fixed count")
        && all_writer_lease_functions == expected_writer_lease_functions;

    if schema_version == 3 && no_memory && no_writer_lease {
        return Ok(CatalogProfile::V3);
    }
    if schema_version == 3 && exact_memory && no_writer_lease {
        return Ok(CatalogProfile::V3CodebaseMemoryV2);
    }
    if schema_version == 3 && exact_memory && exact_writer_lease {
        return Ok(CatalogProfile::V3CodebaseMemoryV2WriterLeaseV1);
    }
    if schema_version == 4 && no_memory && no_writer_lease {
        return Ok(CatalogProfile::V4);
    }
    Err(catalog_error())
}

fn classify_current_catalog_profile<C: GenericClient>(
    client: &mut C,
    schema_version: u16,
) -> Result<CatalogProfile, PostgresStoreSetupError> {
    let row = client
        .query_one(
            "SELECT \
             count(*) FILTER (WHERE c.relname IN ( \
                 'codebase_memory_analyses', \
                 'codebase_memory_extension_identity', \
                 'codebase_memory_extension_ledger', \
                 'codebase_memory_receipts', \
                 'codebase_memory_records', \
                 'codebase_memory_reflections', \
                 'codebase_memory_retrieval_audits', \
                 'openclaw_gateway_commands'))::bigint, \
             count(*)::bigint \
             FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace \
             WHERE n.nspname = 'memory' \
               AND c.relkind IN ('r', 'p', 'v', 'm', 'S', 'f')",
            &[],
        )
        .map_err(|error| map_postgres_error(&error, PostgresStoreSetupErrorKind::CorruptCatalog))?;
    let expected_relations =
        row_value::<i64>(&row, 0, PostgresStoreSetupErrorKind::CorruptCatalog)?;
    let all_relations = row_value::<i64>(&row, 1, PostgresStoreSetupErrorKind::CorruptCatalog)?;
    let row = client
        .query_one(
            "SELECT \
             count(*) FILTER (WHERE p.proname IN ( \
                 'codebase_memory_load_reflection_v2', \
                 'codebase_memory_load_receipt_v1', \
                 'codebase_memory_persist_analysis_v1', \
                 'codebase_memory_persist_reflection_v2', \
                 'codebase_memory_persist_retrieval_v1', \
                 'openclaw_gateway_finalize_terminal_v1', \
                 'openclaw_gateway_reconcile_and_claim_v1'))::bigint, \
             count(*)::bigint \
             FROM pg_proc p JOIN pg_namespace n ON n.oid = p.pronamespace \
             WHERE n.nspname = 'memory'",
            &[],
        )
        .map_err(|error| map_postgres_error(&error, PostgresStoreSetupErrorKind::CorruptCatalog))?;
    let writer_lease_counts = writer_lease_catalog_counts(client)?;
    classify_extension_catalog_counts(
        schema_version,
        expected_relations,
        all_relations,
        row_value::<i64>(&row, 0, PostgresStoreSetupErrorKind::CorruptCatalog)?,
        row_value::<i64>(&row, 1, PostgresStoreSetupErrorKind::CorruptCatalog)?,
        writer_lease_counts.0,
        writer_lease_counts.1,
        writer_lease_counts.2,
        writer_lease_counts.3,
        writer_lease_counts.4,
    )
}

fn writer_lease_catalog_counts<C: GenericClient>(
    client: &mut C,
) -> Result<(i64, i64, i64, i64, i64), PostgresStoreSetupError> {
    let row = client
        .query_one(
            "SELECT \
             (SELECT count(*)::bigint FROM pg_namespace \
               WHERE nspname = 'writer_lease'), \
             count(*) FILTER (WHERE c.relname IN ( \
                 'writer_lease_commands', \
                 'writer_lease_extension_identity', \
                 'writer_lease_extension_ledger', \
                 'writer_lease_heads', \
                 'writer_lease_transitions'))::bigint, \
             count(*)::bigint, \
             (SELECT count(*) FILTER (WHERE p.proname IN ( \
                 'writer_lease_assert_current_v1', \
                 'writer_lease_bind_runtime_v1', \
                 'writer_lease_commit_plan_v1', \
                 'writer_lease_load_commands_v1', \
                 'writer_lease_load_current_v1', \
                 'writer_lease_load_for_update_v1', \
                 'writer_lease_load_transitions_v1'))::bigint \
                FROM pg_proc p JOIN pg_namespace function_ns ON function_ns.oid = p.pronamespace \
               WHERE function_ns.nspname = 'writer_lease'), \
             (SELECT count(*)::bigint \
                FROM pg_proc p JOIN pg_namespace function_ns ON function_ns.oid = p.pronamespace \
               WHERE function_ns.nspname = 'writer_lease') \
             FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace \
             WHERE n.nspname = 'writer_lease' \
               AND c.relkind IN ('r', 'p', 'v', 'm', 'S', 'f')",
            &[],
        )
        .map_err(|error| map_postgres_error(&error, PostgresStoreSetupErrorKind::CorruptCatalog))?;
    Ok((
        row_value::<i64>(&row, 0, PostgresStoreSetupErrorKind::CorruptCatalog)?,
        row_value::<i64>(&row, 1, PostgresStoreSetupErrorKind::CorruptCatalog)?,
        row_value::<i64>(&row, 2, PostgresStoreSetupErrorKind::CorruptCatalog)?,
        row_value::<i64>(&row, 3, PostgresStoreSetupErrorKind::CorruptCatalog)?,
        row_value::<i64>(&row, 4, PostgresStoreSetupErrorKind::CorruptCatalog)?,
    ))
}

/// Applies only the exact embedded executable manifest under the migration role.
///
/// # Errors
///
/// Fails closed with a bounded static error before mutation for an invalid
/// manifest, target, role, server, network, or setting. Pre-commit transaction
/// failures roll back; a commit error is reported as an unknown outcome. Once
/// commit succeeds, a verifier failure is reported separately as committed but
/// unverified and exact manifest retry is required for reconciliation.
pub fn apply_migrations(
    client: &mut Client,
    target: &MigrationTarget,
) -> Result<MigrationApplyOutcome, PostgresStoreSetupError> {
    let manifest = verify_embedded_manifest()?;
    let legacy_manifest = verify_v1_manifest_prefix()?;
    let store_v2_manifest = verify_v2_manifest_prefix()?;
    let v3_manifest = verify_v3_manifest_prefix()?;
    let mut transaction = client
        .build_transaction()
        .isolation_level(IsolationLevel::ReadCommitted)
        .start()
        .map_err(|error| {
            map_postgres_error(&error, PostgresStoreSetupErrorKind::TransactionFailed)
        })?;
    harden_transaction(&mut transaction)?;
    transaction
        .query_one(
            "SELECT pg_advisory_xact_lock($1)",
            &[&MIGRATION_ADVISORY_LOCK],
        )
        .map_err(|error| {
            map_postgres_error(&error, PostgresStoreSetupErrorKind::TransactionFailed)
        })?;
    let connection = preflight_connection(
        &mut transaction,
        target,
        DatabaseRole::Migrator,
        SetupOperation::Migration,
    )?;

    let installed = classify_installed_manifest_state(&mut transaction)?;
    let outcome = match installed {
        InstalledManifestState::Fresh => {
            verify_role_and_database_boundary(&mut transaction, CatalogProfile::PreSchema)?;
            let executable_count = apply_missing_entries(&mut transaction, 0)?;
            seed_database_identity(&mut transaction, target)?;
            insert_current_compatibility(&mut transaction, &manifest)?;
            MigrationApplyOutcome::Applied { executable_count }
        }
        InstalledManifestState::ExactV1Prefix => {
            verify_v1_upgrade_source(&mut transaction, &legacy_manifest, target)?;
            let executable_count = apply_missing_entries(&mut transaction, 2)?;
            advance_compatibility_from_v1(&mut transaction, &legacy_manifest, &manifest)?;
            MigrationApplyOutcome::Applied { executable_count }
        }
        InstalledManifestState::ExactV2Prefix => {
            verify_v2_upgrade_source(&mut transaction, &store_v2_manifest, target)?;
            let executable_count = apply_missing_entries(&mut transaction, 3)?;
            advance_compatibility_from_v2(&mut transaction, &store_v2_manifest, &manifest)?;
            MigrationApplyOutcome::Applied { executable_count }
        }
        InstalledManifestState::ExactV3Prefix => {
            verify_v3_upgrade_source(&mut transaction, &v3_manifest, target)?;
            let executable_count = apply_missing_entries(&mut transaction, 4)?;
            advance_compatibility_from_v3(&mut transaction, &v3_manifest, &manifest)?;
            MigrationApplyOutcome::Applied { executable_count }
        }
        InstalledManifestState::ExactV4Full => {
            verify_catalog(
                &mut transaction,
                &manifest,
                target,
                DatabaseRole::Migrator,
                connection.server_version_num,
            )?;
            MigrationApplyOutcome::AlreadyCurrent
        }
    };

    preflight_connection(
        &mut transaction,
        target,
        DatabaseRole::Migrator,
        SetupOperation::Migration,
    )?;
    let current_profile =
        classify_current_catalog_profile(&mut transaction, POSTGRES_SCHEMA_VERSION)?;
    verify_role_and_database_boundary(&mut transaction, current_profile)?;

    transaction.commit().map_err(|_| {
        PostgresStoreSetupError::new(PostgresStoreSetupErrorKind::CommitOutcomeUnknown)
    })?;
    verify_postgres_schema(client, target, DatabaseRole::Migrator).map_err(|_| {
        PostgresStoreSetupError::new(PostgresStoreSetupErrorKind::PostApplyVerificationFailed)
    })?;
    Ok(outcome)
}

/// Verifies exact target, role, settings, manifest history, catalog, grants,
/// and STOPPED/no-leader bootstrap without applying a migration.
///
/// # Errors
///
/// Returns a bounded static failure for any mismatch or unavailable evidence.
pub fn verify_postgres_schema(
    client: &mut Client,
    target: &MigrationTarget,
    role: DatabaseRole,
) -> Result<PostgresSchemaEvidence, PostgresStoreSetupError> {
    let manifest = verify_embedded_manifest()?;
    let mut transaction = client
        .build_transaction()
        .isolation_level(IsolationLevel::RepeatableRead)
        .read_only(true)
        .start()
        .map_err(|error| {
            map_postgres_error(&error, PostgresStoreSetupErrorKind::TransactionFailed)
        })?;
    harden_transaction(&mut transaction)?;
    let connection =
        preflight_connection(&mut transaction, target, role, SetupOperation::Verification)?;
    if owned_schema_presence(&mut transaction)? != [true, true, true] {
        return Err(PostgresStoreSetupError::new(
            PostgresStoreSetupErrorKind::HistoryMismatch,
        ));
    }
    let evidence = verify_catalog(
        &mut transaction,
        &manifest,
        target,
        role,
        connection.server_version_num,
    )?;
    preflight_connection(&mut transaction, target, role, SetupOperation::Verification)?;
    transaction.commit().map_err(|error| {
        map_postgres_error(&error, PostgresStoreSetupErrorKind::TransactionFailed)
    })?;
    Ok(evidence)
}

pub(crate) fn verify_runtime_store_schema(
    client: &mut Client,
    target: &MigrationTarget,
) -> Result<RuntimeStoreSchemaEvidence, PostgresStoreSetupError> {
    let store_v2_manifest = verify_v2_manifest_prefix()?;
    let mut transaction = client
        .build_transaction()
        .isolation_level(IsolationLevel::RepeatableRead)
        .read_only(true)
        .start()
        .map_err(|error| {
            map_postgres_error(&error, PostgresStoreSetupErrorKind::TransactionFailed)
        })?;
    harden_transaction(&mut transaction)?;
    preflight_connection(
        &mut transaction,
        target,
        DatabaseRole::Runtime,
        SetupOperation::Verification,
    )?;
    if owned_schema_presence(&mut transaction)? != [true, true, true] {
        return Err(history_error());
    }
    let compatibility = transaction
        .query(
            "SELECT current_schema_version FROM ONLY control.schema_compatibility \
             WHERE singleton = true",
            &[],
        )
        .map_err(|error| map_postgres_error(&error, PostgresStoreSetupErrorKind::CorruptCatalog))?;
    if compatibility.len() != 1 {
        return Err(catalog_error());
    }
    let installed_schema_version: i16 = row_value(
        &compatibility[0],
        0,
        PostgresStoreSetupErrorKind::CompatibilityMismatch,
    )?;
    let installed_schema_version = u16::try_from(installed_schema_version).map_err(|_| {
        PostgresStoreSetupError::new(PostgresStoreSetupErrorKind::CompatibilityMismatch)
    })?;
    let manifest = match installed_schema_version {
        3 => verify_v3_manifest_prefix()?,
        POSTGRES_SCHEMA_VERSION => verify_embedded_manifest()?,
        _ => {
            return Err(PostgresStoreSetupError::new(
                PostgresStoreSetupErrorKind::CompatibilityMismatch,
            ));
        }
    };
    let current_profile =
        classify_current_catalog_profile(&mut transaction, installed_schema_version)?;
    verify_schema_objects(&mut transaction, current_profile)?;
    let rows = read_history_rows(&mut transaction)?;
    let expected_history = if installed_schema_version == 3 {
        &migration_manifest()[..4]
    } else {
        migration_manifest()
    };
    verify_history_rows(&rows, expected_history)?;
    verify_compatibility(&mut transaction, &manifest, current_profile)?;
    let database_uuid = read_database_identity(&mut transaction, target)?;
    verify_runtime_admission_present(&mut transaction)?;
    verify_roles_and_grants(&mut transaction, current_profile)?;
    preflight_connection(
        &mut transaction,
        target,
        DatabaseRole::Runtime,
        SetupOperation::Verification,
    )?;
    transaction.commit().map_err(|error| {
        map_postgres_error(&error, PostgresStoreSetupErrorKind::TransactionFailed)
    })?;
    Ok(RuntimeStoreSchemaEvidence {
        database_uuid,
        global_manifest_sha256: manifest.manifest_sha256().clone(),
        global_schema_version: installed_schema_version,
        store_manifest_sha256: store_v2_manifest.manifest_sha256().clone(),
        store_schema_version: STORE_V2_SCHEMA_VERSION,
    })
}

fn preflight_connection<C: GenericClient>(
    client: &mut C,
    target: &MigrationTarget,
    role: DatabaseRole,
    operation: SetupOperation,
) -> Result<ConnectionEvidence, PostgresStoreSetupError> {
    let row = client
        .query_one(
            "SELECT current_database()::text, \
             (SELECT shobj_description(oid, 'pg_database') FROM pg_database \
              WHERE datname = current_database()), \
             current_user::text, session_user::text, \
             inet_server_addr()::text, inet_client_addr()::text, \
             current_setting('server_version_num'), current_setting('server_encoding'), \
             current_setting('fsync'), current_setting('synchronous_commit'), \
             current_setting('full_page_writes'), current_setting('data_checksums'), \
             current_setting('max_prepared_transactions'), \
             current_setting('application_name'), \
             COALESCE((SELECT ssl FROM pg_stat_ssl WHERE pid = pg_backend_pid()), false), \
             current_setting('transaction_isolation'), \
             current_setting('transaction_read_only')",
            &[],
        )
        .map_err(|error| {
            map_postgres_error(&error, PostgresStoreSetupErrorKind::TransactionFailed)
        })?;

    let database_name = row_value::<String>(&row, 0, PostgresStoreSetupErrorKind::TargetMismatch)?;
    let database_comment =
        row_value::<Option<String>>(&row, 1, PostgresStoreSetupErrorKind::TargetUnowned)?;
    let current_role = row_value::<String>(&row, 2, PostgresStoreSetupErrorKind::PermissionDenied)?;
    let session_role = row_value::<String>(&row, 3, PostgresStoreSetupErrorKind::PermissionDenied)?;
    let server_address =
        row_value::<Option<String>>(&row, 4, PostgresStoreSetupErrorKind::NetworkBoundary)?;
    let client_address =
        row_value::<Option<String>>(&row, 5, PostgresStoreSetupErrorKind::NetworkBoundary)?;
    let server_version_text =
        row_value::<String>(&row, 6, PostgresStoreSetupErrorKind::ServerUnsupported)?;
    let server_encoding = row_value::<String>(&row, 7, PostgresStoreSetupErrorKind::UnsafeSetting)?;
    let fsync = row_value::<String>(&row, 8, PostgresStoreSetupErrorKind::UnsafeSetting)?;
    let synchronous_commit =
        row_value::<String>(&row, 9, PostgresStoreSetupErrorKind::UnsafeSetting)?;
    let full_page_writes =
        row_value::<String>(&row, 10, PostgresStoreSetupErrorKind::UnsafeSetting)?;
    let data_checksums = row_value::<String>(&row, 11, PostgresStoreSetupErrorKind::UnsafeSetting)?;
    let max_prepared_transactions =
        row_value::<String>(&row, 12, PostgresStoreSetupErrorKind::UnsafeSetting)?;
    let application_name =
        row_value::<String>(&row, 13, PostgresStoreSetupErrorKind::UnsafeSetting)?;
    let ssl = row_value::<bool>(&row, 14, PostgresStoreSetupErrorKind::NetworkBoundary)?;
    let transaction_isolation =
        row_value::<String>(&row, 15, PostgresStoreSetupErrorKind::UnsafeSetting)?;
    let transaction_read_only =
        row_value::<String>(&row, 16, PostgresStoreSetupErrorKind::UnsafeSetting)?;

    if database_name != target.database_name() {
        return Err(PostgresStoreSetupError::new(
            PostgresStoreSetupErrorKind::TargetMismatch,
        ));
    }
    if database_comment.as_deref() != Some(target.database_comment().as_str()) {
        return Err(PostgresStoreSetupError::new(
            PostgresStoreSetupErrorKind::TargetUnowned,
        ));
    }
    if current_role != role.as_str() || session_role != role.login_role() {
        return Err(PostgresStoreSetupError::new(
            PostgresStoreSetupErrorKind::PermissionDenied,
        ));
    }
    verify_network_boundary(server_address.as_deref(), client_address.as_deref(), ssl)?;
    let server_version_num = server_version_text.parse::<u32>().map_err(|_| {
        PostgresStoreSetupError::new(PostgresStoreSetupErrorKind::ServerUnsupported)
    })?;
    verify_server_version(server_version_num)?;
    if server_encoding != "UTF8"
        || fsync != "on"
        || synchronous_commit != "on"
        || full_page_writes != "on"
        || data_checksums != "on"
        || max_prepared_transactions != "0"
        || application_name != REQUIRED_APPLICATION_NAME
        || match operation {
            SetupOperation::Migration => {
                transaction_isolation != "read committed" || transaction_read_only != "off"
            }
            SetupOperation::Verification => {
                transaction_isolation != "repeatable read" || transaction_read_only != "on"
            }
        }
    {
        return Err(PostgresStoreSetupError::new(
            PostgresStoreSetupErrorKind::UnsafeSetting,
        ));
    }
    Ok(ConnectionEvidence { server_version_num })
}

fn verify_network_boundary(
    server_address: Option<&str>,
    client_address: Option<&str>,
    ssl: bool,
) -> Result<(), PostgresStoreSetupError> {
    if !server_address.is_some_and(is_loopback) || !client_address.is_some_and(is_loopback) || ssl {
        return Err(PostgresStoreSetupError::new(
            PostgresStoreSetupErrorKind::NetworkBoundary,
        ));
    }
    Ok(())
}

fn verify_server_version(server_version_num: u32) -> Result<(), PostgresStoreSetupError> {
    if server_version_num / 10_000 != SUPPORTED_POSTGRES_MAJOR {
        return Err(PostgresStoreSetupError::new(
            PostgresStoreSetupErrorKind::ServerUnsupported,
        ));
    }
    Ok(())
}

fn is_loopback(address: &str) -> bool {
    matches!(address, "127.0.0.1" | "127.0.0.1/32" | "::1" | "::1/128")
}

fn harden_transaction<C: GenericClient>(client: &mut C) -> Result<(), PostgresStoreSetupError> {
    client
        .batch_execute("SET LOCAL search_path = pg_catalog; SET LOCAL row_security = on")
        .map_err(|error| map_postgres_error(&error, PostgresStoreSetupErrorKind::TransactionFailed))
}

fn owned_schema_presence<C: GenericClient>(
    client: &mut C,
) -> Result<[bool; 3], PostgresStoreSetupError> {
    let row = client
        .query_one(
            "SELECT to_regnamespace('control') IS NOT NULL, \
             to_regnamespace('memory') IS NOT NULL, \
             to_regnamespace('readmodel') IS NOT NULL",
            &[],
        )
        .map_err(|error| {
            map_postgres_error(&error, PostgresStoreSetupErrorKind::TransactionFailed)
        })?;
    Ok([row.get(0), row.get(1), row.get(2)])
}

fn classify_installed_manifest_state<C: GenericClient>(
    client: &mut C,
) -> Result<InstalledManifestState, PostgresStoreSetupError> {
    match owned_schema_presence(client)? {
        [false, false, false] => Ok(InstalledManifestState::Fresh),
        [true, true, true] => {
            let rows = read_history_rows(client)?;
            match rows.len() {
                2 => {
                    verify_history_rows(&rows, &migration_manifest()[..2])?;
                    Ok(InstalledManifestState::ExactV1Prefix)
                }
                3 => {
                    verify_history_rows(&rows, &migration_manifest()[..3])?;
                    Ok(InstalledManifestState::ExactV2Prefix)
                }
                4 => {
                    verify_history_rows(&rows, &migration_manifest()[..4])?;
                    Ok(InstalledManifestState::ExactV3Prefix)
                }
                length if length == migration_manifest().len() => {
                    verify_history_rows(&rows, migration_manifest())?;
                    Ok(InstalledManifestState::ExactV4Full)
                }
                _ => Err(history_error()),
            }
        }
        _ => Err(PostgresStoreSetupError::new(
            PostgresStoreSetupErrorKind::SchemaCollision,
        )),
    }
}

fn apply_missing_entries<C: GenericClient>(
    client: &mut C,
    applied_prefix_len: usize,
) -> Result<usize, PostgresStoreSetupError> {
    if applied_prefix_len > migration_manifest().len() {
        return Err(history_error());
    }
    let mut executable_count = 0usize;
    for entry in &migration_manifest()[applied_prefix_len..] {
        if entry.status() == MigrationStatus::Executable {
            let sql = std::str::from_utf8(entry.bytes()).map_err(|_| {
                PostgresStoreSetupError::new(PostgresStoreSetupErrorKind::ManifestInvalid)
            })?;
            client.batch_execute(sql).map_err(|error| {
                map_postgres_error(&error, PostgresStoreSetupErrorKind::TransactionFailed)
            })?;
            executable_count += 1;
        }
    }

    for entry in &migration_manifest()[applied_prefix_len..] {
        insert_history(client, entry)?;
    }
    Ok(executable_count)
}

fn insert_current_compatibility<C: GenericClient>(
    client: &mut C,
    manifest: &ManifestEvidence,
) -> Result<(), PostgresStoreSetupError> {
    let current = migration_manifest().last().ok_or_else(|| {
        PostgresStoreSetupError::new(PostgresStoreSetupErrorKind::ManifestInvalid)
    })?;
    client
        .execute(
            "INSERT INTO control.schema_compatibility (\
                 singleton, manifest_sha256, current_schema_version, \
                 min_reader, max_reader, min_writer, max_writer\
             ) VALUES (true, $1, $2, $3, $4, $5, $6)",
            &[
                &manifest.manifest_sha256().as_str(),
                &to_i16(manifest.schema_version())?,
                &to_i16(*current.reader_compatibility().start())?,
                &to_i16(*current.reader_compatibility().end())?,
                &to_i16(*current.writer_compatibility().start())?,
                &to_i16(*current.writer_compatibility().end())?,
            ],
        )
        .map_err(|error| {
            map_postgres_error(&error, PostgresStoreSetupErrorKind::TransactionFailed)
        })?;
    Ok(())
}

fn advance_compatibility_from_v1<C: GenericClient>(
    client: &mut C,
    legacy_manifest: &ManifestEvidence,
    current_manifest: &ManifestEvidence,
) -> Result<(), PostgresStoreSetupError> {
    let updated = client
        .execute(
            "UPDATE ONLY control.schema_compatibility \
             SET manifest_sha256 = $1, current_schema_version = $2, \
                 min_reader = $3, max_reader = $4, min_writer = $5, max_writer = $6, \
                 updated_at = clock_timestamp() \
             WHERE singleton = true \
               AND manifest_sha256 = $7 \
               AND current_schema_version = 1 \
               AND min_reader = 1 AND max_reader = 1 \
               AND min_writer = 1 AND max_writer = 1",
            &[
                &current_manifest.manifest_sha256().as_str(),
                &4_i16,
                &4_i16,
                &4_i16,
                &4_i16,
                &4_i16,
                &legacy_manifest.manifest_sha256().as_str(),
            ],
        )
        .map_err(|error| {
            map_postgres_error(&error, PostgresStoreSetupErrorKind::CompatibilityMismatch)
        })?;
    if updated != 1 {
        return Err(PostgresStoreSetupError::new(
            PostgresStoreSetupErrorKind::CompatibilityMismatch,
        ));
    }
    Ok(())
}

fn advance_compatibility_from_v2<C: GenericClient>(
    client: &mut C,
    store_v2_manifest: &ManifestEvidence,
    current_manifest: &ManifestEvidence,
) -> Result<(), PostgresStoreSetupError> {
    let updated = client
        .execute(
            "UPDATE ONLY control.schema_compatibility \
             SET manifest_sha256 = $1, current_schema_version = 4, \
                 min_reader = 4, max_reader = 4, min_writer = 4, max_writer = 4, \
                 updated_at = clock_timestamp() \
             WHERE singleton = true \
               AND manifest_sha256 = $2 \
               AND current_schema_version = 2 \
               AND min_reader = 2 AND max_reader = 2 \
               AND min_writer = 2 AND max_writer = 2",
            &[
                &current_manifest.manifest_sha256().as_str(),
                &store_v2_manifest.manifest_sha256().as_str(),
            ],
        )
        .map_err(|error| {
            map_postgres_error(&error, PostgresStoreSetupErrorKind::CompatibilityMismatch)
        })?;
    if updated != 1 {
        return Err(PostgresStoreSetupError::new(
            PostgresStoreSetupErrorKind::CompatibilityMismatch,
        ));
    }
    Ok(())
}

fn advance_compatibility_from_v3<C: GenericClient>(
    client: &mut C,
    v3_manifest: &ManifestEvidence,
    current_manifest: &ManifestEvidence,
) -> Result<(), PostgresStoreSetupError> {
    let updated = client
        .execute(
            "UPDATE ONLY control.schema_compatibility \
             SET manifest_sha256 = $1, current_schema_version = 4, \
                 min_reader = 4, max_reader = 4, min_writer = 4, max_writer = 4, \
                 updated_at = clock_timestamp() \
             WHERE singleton = true \
               AND manifest_sha256 = $2 \
               AND current_schema_version = 3 \
               AND min_reader = 3 AND max_reader = 3 \
               AND min_writer = 3 AND max_writer = 3",
            &[
                &current_manifest.manifest_sha256().as_str(),
                &v3_manifest.manifest_sha256().as_str(),
            ],
        )
        .map_err(|error| {
            map_postgres_error(&error, PostgresStoreSetupErrorKind::CompatibilityMismatch)
        })?;
    if updated != 1 {
        return Err(PostgresStoreSetupError::new(
            PostgresStoreSetupErrorKind::CompatibilityMismatch,
        ));
    }
    Ok(())
}

fn insert_history<C: GenericClient>(
    client: &mut C,
    entry: &MigrationDescriptor,
) -> Result<(), PostgresStoreSetupError> {
    let ordinal = to_i16(entry.ordinal())?;
    let byte_length = i64::try_from(entry.byte_length())
        .map_err(|_| PostgresStoreSetupError::new(PostgresStoreSetupErrorKind::ManifestInvalid))?;
    let schema_version = to_i16(entry.schema_version())?;
    let min_reader = to_i16(*entry.reader_compatibility().start())?;
    let max_reader = to_i16(*entry.reader_compatibility().end())?;
    let min_writer = to_i16(*entry.writer_compatibility().start())?;
    let max_writer = to_i16(*entry.writer_compatibility().end())?;
    client
        .execute(
            "INSERT INTO control.migration_history (\
                 ordinal, migration_id, migration_path, byte_length, checksum_sha256, \
                 migration_status, transaction_mode, schema_version, min_reader, \
                 max_reader, min_writer, max_writer\
             ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)",
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
        .map_err(|error| {
            map_postgres_error(&error, PostgresStoreSetupErrorKind::TransactionFailed)
        })?;
    Ok(())
}

fn seed_database_identity<C: GenericClient>(
    client: &mut C,
    target: &MigrationTarget,
) -> Result<(), PostgresStoreSetupError> {
    let inserted = client
        .execute(
            "INSERT INTO control.database_identity (singleton, database_uuid) \
             VALUES (true, $1::text::uuid)",
            &[&target.expected_database_uuid()],
        )
        .map_err(|error| {
            map_postgres_error(&error, PostgresStoreSetupErrorKind::TransactionFailed)
        })?;
    if inserted != 1 {
        return Err(PostgresStoreSetupError::new(
            PostgresStoreSetupErrorKind::TransactionFailed,
        ));
    }
    Ok(())
}

fn verify_catalog<C: GenericClient>(
    client: &mut C,
    manifest: &ManifestEvidence,
    target: &MigrationTarget,
    role: DatabaseRole,
    server_version_num: u32,
) -> Result<PostgresSchemaEvidence, PostgresStoreSetupError> {
    let current_profile = classify_current_catalog_profile(client, POSTGRES_SCHEMA_VERSION)?;
    verify_schema_objects(client, current_profile)?;
    verify_history(client)?;
    verify_compatibility(client, manifest, current_profile)?;
    let database_uuid = read_database_identity(client, target)?;
    verify_stopped_admission(client)?;
    verify_roles_and_grants(client, current_profile)?;

    Ok(PostgresSchemaEvidence {
        database_uuid,
        manifest_sha256: manifest.manifest_sha256().clone(),
        schema_version: POSTGRES_SCHEMA_VERSION,
        server_version_num,
        role,
        bootstrap_admission: BootstrapAdmission::StoppedNoLeader,
    })
}

fn verify_v1_upgrade_source<C: GenericClient>(
    client: &mut C,
    legacy_manifest: &ManifestEvidence,
    target: &MigrationTarget,
) -> Result<(), PostgresStoreSetupError> {
    client
        .batch_execute(
            "LOCK TABLE control.runtime_admission IN ACCESS EXCLUSIVE MODE; \
             LOCK TABLE control.physical_heads IN ACCESS EXCLUSIVE MODE; \
             LOCK TABLE control.terminal_transactions IN ACCESS EXCLUSIVE MODE",
        )
        .map_err(|error| {
            map_postgres_error(&error, PostgresStoreSetupErrorKind::TransactionFailed)
        })?;
    verify_schema_objects(client, CatalogProfile::V1)?;
    let rows = read_history_rows(client)?;
    verify_history_rows(&rows, &migration_manifest()[..2])?;
    verify_compatibility(client, legacy_manifest, CatalogProfile::V1)?;
    read_database_identity(client, target)?;
    verify_stopped_admission(client)?;
    verify_roles_and_grants(client, CatalogProfile::V1)?;
    verify_v1_store_empty(client)
}

fn verify_v2_upgrade_source<C: GenericClient>(
    client: &mut C,
    store_v2_manifest: &ManifestEvidence,
    target: &MigrationTarget,
) -> Result<(), PostgresStoreSetupError> {
    client
        .batch_execute(
            "LOCK TABLE control.runtime_admission IN ACCESS EXCLUSIVE MODE; \
             LOCK TABLE control.physical_heads IN ACCESS EXCLUSIVE MODE; \
             LOCK TABLE control.terminal_transactions IN ACCESS EXCLUSIVE MODE",
        )
        .map_err(|error| {
            map_postgres_error(&error, PostgresStoreSetupErrorKind::TransactionFailed)
        })?;
    verify_schema_objects(client, CatalogProfile::V2)?;
    let rows = read_history_rows(client)?;
    verify_history_rows(&rows, &migration_manifest()[..3])?;
    verify_compatibility(client, store_v2_manifest, CatalogProfile::V2)?;
    read_database_identity(client, target)?;
    verify_stopped_admission(client)?;
    verify_roles_and_grants(client, CatalogProfile::V2)
}

fn verify_v3_upgrade_source<C: GenericClient>(
    client: &mut C,
    v3_manifest: &ManifestEvidence,
    target: &MigrationTarget,
) -> Result<(), PostgresStoreSetupError> {
    client
        .batch_execute(
            "LOCK TABLE control.runtime_admission IN ACCESS EXCLUSIVE MODE; \
             LOCK TABLE control.physical_heads IN ACCESS EXCLUSIVE MODE; \
             LOCK TABLE control.terminal_transactions IN ACCESS EXCLUSIVE MODE; \
             LOCK TABLE control.task_ledger_streams IN ACCESS EXCLUSIVE MODE; \
             LOCK TABLE control.task_ledger_commands IN ACCESS EXCLUSIVE MODE; \
             LOCK TABLE control.task_ledger_events IN ACCESS EXCLUSIVE MODE; \
             LOCK TABLE control.task_ledger_outbox IN ACCESS EXCLUSIVE MODE",
        )
        .map_err(|error| {
            map_postgres_error(&error, PostgresStoreSetupErrorKind::TransactionFailed)
        })?;
    let profile = classify_current_catalog_profile(client, 3)?;
    if profile != CatalogProfile::V3 {
        return Err(PostgresStoreSetupError::new(
            PostgresStoreSetupErrorKind::CompatibilityMismatch,
        ));
    }
    verify_schema_objects(client, profile)?;
    let rows = read_history_rows(client)?;
    verify_history_rows(&rows, &migration_manifest()[..4])?;
    verify_compatibility(client, v3_manifest, profile)?;
    read_database_identity(client, target)?;
    verify_stopped_admission(client)?;
    verify_roles_and_grants(client, profile)
}

fn verify_v1_store_empty<C: GenericClient>(client: &mut C) -> Result<(), PostgresStoreSetupError> {
    let row = client
        .query_one(
            "SELECT \
             (SELECT count(*) FROM ONLY control.physical_heads), \
             (SELECT count(*) FROM ONLY control.terminal_transactions)",
            &[],
        )
        .map_err(|error| {
            map_postgres_error(&error, PostgresStoreSetupErrorKind::CompatibilityMismatch)
        })?;
    if row_value::<i64>(&row, 0, PostgresStoreSetupErrorKind::CompatibilityMismatch)? != 0
        || row_value::<i64>(&row, 1, PostgresStoreSetupErrorKind::CompatibilityMismatch)? != 0
    {
        return Err(PostgresStoreSetupError::new(
            PostgresStoreSetupErrorKind::CompatibilityMismatch,
        ));
    }
    Ok(())
}

fn verify_compatibility<C: GenericClient>(
    client: &mut C,
    manifest: &ManifestEvidence,
    profile: CatalogProfile,
) -> Result<(), PostgresStoreSetupError> {
    let compatibility = client
        .query(
            "SELECT manifest_sha256, current_schema_version, min_reader, \
             max_reader, min_writer, max_writer \
             FROM ONLY control.schema_compatibility WHERE singleton = true",
            &[],
        )
        .map_err(|error| map_postgres_error(&error, PostgresStoreSetupErrorKind::CorruptCatalog))?;
    if compatibility.len() != 1 {
        return Err(catalog_error());
    }
    let row = &compatibility[0];
    let manifest_digest =
        row_value::<String>(row, 0, PostgresStoreSetupErrorKind::CompatibilityMismatch)?;
    let versions: [i16; 5] = [
        row_value(row, 1, PostgresStoreSetupErrorKind::CompatibilityMismatch)?,
        row_value(row, 2, PostgresStoreSetupErrorKind::CompatibilityMismatch)?,
        row_value(row, 3, PostgresStoreSetupErrorKind::CompatibilityMismatch)?,
        row_value(row, 4, PostgresStoreSetupErrorKind::CompatibilityMismatch)?,
        row_value(row, 5, PostgresStoreSetupErrorKind::CompatibilityMismatch)?,
    ];
    let expected_versions = match profile {
        CatalogProfile::V1 => [1, 1, 1, 1, 1],
        CatalogProfile::V2 => [2, 2, 2, 2, 2],
        CatalogProfile::V3
        | CatalogProfile::V3CodebaseMemoryV2
        | CatalogProfile::V3CodebaseMemoryV2WriterLeaseV1 => [3, 3, 3, 3, 3],
        CatalogProfile::V4 => [4, 4, 4, 4, 4],
        CatalogProfile::PreSchema => {
            return Err(PostgresStoreSetupError::new(
                PostgresStoreSetupErrorKind::CompatibilityMismatch,
            ));
        }
    };
    if manifest_digest != manifest.manifest_sha256().as_str() || versions != expected_versions {
        return Err(PostgresStoreSetupError::new(
            PostgresStoreSetupErrorKind::CompatibilityMismatch,
        ));
    }
    Ok(())
}

fn read_database_identity<C: GenericClient>(
    client: &mut C,
    target: &MigrationTarget,
) -> Result<String, PostgresStoreSetupError> {
    let identities = client
        .query(
            "SELECT database_uuid::text FROM ONLY control.database_identity WHERE singleton = true",
            &[],
        )
        .map_err(|error| map_postgres_error(&error, PostgresStoreSetupErrorKind::CorruptCatalog))?;
    if identities.len() != 1 {
        return Err(catalog_error());
    }
    let database_uuid = row_value::<String>(
        &identities[0],
        0,
        PostgresStoreSetupErrorKind::CorruptCatalog,
    )?;
    if !is_canonical_uuid(&database_uuid) || database_uuid != target.expected_database_uuid() {
        return Err(catalog_error());
    }
    Ok(database_uuid)
}

fn verify_stopped_admission<C: GenericClient>(
    client: &mut C,
) -> Result<(), PostgresStoreSetupError> {
    let admission = client
        .query(
            "SELECT admission_mode, daemon_instance_id, daemon_epoch, \
             authority_revision, observation_digest, authority_head_digest \
             FROM ONLY control.runtime_admission WHERE singleton = true",
            &[],
        )
        .map_err(|error| map_postgres_error(&error, PostgresStoreSetupErrorKind::CorruptCatalog))?;
    if admission.len() != 1 {
        return Err(catalog_error());
    }
    let admission_row = &admission[0];
    let mode = row_value::<String>(
        admission_row,
        0,
        PostgresStoreSetupErrorKind::CompatibilityMismatch,
    )?;
    let instance = row_value::<Option<String>>(
        admission_row,
        1,
        PostgresStoreSetupErrorKind::CompatibilityMismatch,
    )?;
    let epoch = row_value::<Option<i64>>(
        admission_row,
        2,
        PostgresStoreSetupErrorKind::CompatibilityMismatch,
    )?;
    let revision = row_value::<i64>(
        admission_row,
        3,
        PostgresStoreSetupErrorKind::CompatibilityMismatch,
    )?;
    let observation = row_value::<Option<Vec<u8>>>(
        admission_row,
        4,
        PostgresStoreSetupErrorKind::CompatibilityMismatch,
    )?;
    let head = row_value::<Option<Vec<u8>>>(
        admission_row,
        5,
        PostgresStoreSetupErrorKind::CompatibilityMismatch,
    )?;
    if mode != "STOPPED"
        || instance.is_some()
        || epoch.is_some()
        || revision != 0
        || observation.is_some()
        || head.is_some()
    {
        return Err(PostgresStoreSetupError::new(
            PostgresStoreSetupErrorKind::CompatibilityMismatch,
        ));
    }
    Ok(())
}

fn verify_runtime_admission_present<C: GenericClient>(
    client: &mut C,
) -> Result<(), PostgresStoreSetupError> {
    let rows = client
        .query(
            "SELECT admission_mode, daemon_instance_id, daemon_epoch, \
             authority_revision, observation_digest, authority_head_digest \
             FROM ONLY control.runtime_admission WHERE singleton = true",
            &[],
        )
        .map_err(|error| map_postgres_error(&error, PostgresStoreSetupErrorKind::CorruptCatalog))?;
    if rows.len() != 1 {
        return Err(catalog_error());
    }
    let row = &rows[0];
    let mode = row_value::<String>(row, 0, PostgresStoreSetupErrorKind::CorruptCatalog)?;
    let daemon = row_value::<Option<String>>(row, 1, PostgresStoreSetupErrorKind::CorruptCatalog)?;
    let epoch = row_value::<Option<i64>>(row, 2, PostgresStoreSetupErrorKind::CorruptCatalog)?;
    let revision = row_value::<i64>(row, 3, PostgresStoreSetupErrorKind::CorruptCatalog)?;
    let observation =
        row_value::<Option<Vec<u8>>>(row, 4, PostgresStoreSetupErrorKind::CorruptCatalog)?;
    let head = row_value::<Option<Vec<u8>>>(row, 5, PostgresStoreSetupErrorKind::CorruptCatalog)?;
    let valid = if mode == "STOPPED" {
        daemon.is_none()
            && epoch.is_none()
            && revision == 0
            && observation.is_none()
            && head.is_none()
    } else {
        matches!(
            mode.as_str(),
            "ACTIVE" | "DRAINING" | "CANARY" | "RECONCILIATION_REQUIRED"
        ) && daemon.is_some()
            && epoch.is_some_and(|value| value > 0)
            && revision > 0
            && observation.as_ref().is_some_and(|value| value.len() == 32)
            && head.as_ref().is_some_and(|value| value.len() == 32)
    };
    if !valid {
        return Err(catalog_error());
    }
    Ok(())
}

fn verify_history<C: GenericClient>(client: &mut C) -> Result<(), PostgresStoreSetupError> {
    let rows = read_history_rows(client)?;
    verify_history_rows(&rows, migration_manifest())
}

fn read_history_rows<C: GenericClient>(
    client: &mut C,
) -> Result<Vec<Row>, PostgresStoreSetupError> {
    client
        .query(
            "SELECT ordinal, migration_id, migration_path, byte_length, \
             checksum_sha256, migration_status, transaction_mode, schema_version, \
             min_reader, max_reader, min_writer, max_writer \
             FROM ONLY control.migration_history ORDER BY ordinal",
            &[],
        )
        .map_err(|error| map_postgres_error(&error, PostgresStoreSetupErrorKind::HistoryMismatch))
}

fn verify_history_rows(
    rows: &[Row],
    expected: &[MigrationDescriptor],
) -> Result<(), PostgresStoreSetupError> {
    if rows.len() != expected.len() {
        return Err(history_error());
    }
    for (row, entry) in rows.iter().zip(expected) {
        let values = (
            row_value::<i16>(row, 0, PostgresStoreSetupErrorKind::HistoryMismatch)?,
            row_value::<String>(row, 1, PostgresStoreSetupErrorKind::HistoryMismatch)?,
            row_value::<String>(row, 2, PostgresStoreSetupErrorKind::HistoryMismatch)?,
            row_value::<i64>(row, 3, PostgresStoreSetupErrorKind::HistoryMismatch)?,
            row_value::<String>(row, 4, PostgresStoreSetupErrorKind::HistoryMismatch)?,
            row_value::<String>(row, 5, PostgresStoreSetupErrorKind::HistoryMismatch)?,
            row_value::<String>(row, 6, PostgresStoreSetupErrorKind::HistoryMismatch)?,
            row_value::<i16>(row, 7, PostgresStoreSetupErrorKind::HistoryMismatch)?,
            row_value::<i16>(row, 8, PostgresStoreSetupErrorKind::HistoryMismatch)?,
            row_value::<i16>(row, 9, PostgresStoreSetupErrorKind::HistoryMismatch)?,
            row_value::<i16>(row, 10, PostgresStoreSetupErrorKind::HistoryMismatch)?,
            row_value::<i16>(row, 11, PostgresStoreSetupErrorKind::HistoryMismatch)?,
        );
        if values.0 != to_i16(entry.ordinal())?
            || values.1 != entry.id()
            || values.2 != entry.path()
            || values.3 != i64::try_from(entry.byte_length()).map_err(|_| history_error())?
            || values.4 != entry.sha256()
            || values.5 != entry.status().as_str()
            || values.6 != entry.transaction_mode().as_str()
            || values.7 != to_i16(entry.schema_version())?
            || values.8 != to_i16(*entry.reader_compatibility().start())?
            || values.9 != to_i16(*entry.reader_compatibility().end())?
            || values.10 != to_i16(*entry.writer_compatibility().start())?
            || values.11 != to_i16(*entry.writer_compatibility().end())?
        {
            return Err(history_error());
        }
    }
    Ok(())
}

fn verify_schema_objects<C: GenericClient>(
    client: &mut C,
    profile: CatalogProfile,
) -> Result<(), PostgresStoreSetupError> {
    verify_catalog_signatures(client, profile)?;
    verify_schema_headers(client, profile)?;
    verify_forbidden_schema_objects(client, profile)?;
    if matches!(
        profile,
        CatalogProfile::V2
            | CatalogProfile::V3
            | CatalogProfile::V3CodebaseMemoryV2
            | CatalogProfile::V3CodebaseMemoryV2WriterLeaseV1
            | CatalogProfile::V4
    ) {
        verify_owned_function_boundary(client, profile)?;
    }
    if matches!(profile, CatalogProfile::V3CodebaseMemoryV2WriterLeaseV1) {
        verify_writer_lease_v1_profile(client)?;
    }
    verify_forbidden_namespace_objects(client)?;
    Ok(())
}

#[allow(clippy::too_many_lines)]
fn verify_writer_lease_v1_profile<C: GenericClient>(
    client: &mut C,
) -> Result<(), PostgresStoreSetupError> {
    let header = client
        .query_one(
            "SELECT owner.rolname, pg_catalog.obj_description(n.oid, 'pg_namespace') \
               FROM pg_catalog.pg_namespace n \
               JOIN pg_catalog.pg_roles owner ON owner.oid = n.nspowner \
              WHERE n.nspname = 'writer_lease'",
            &[],
        )
        .map_err(|error| map_postgres_error(&error, PostgresStoreSetupErrorKind::CorruptCatalog))?;
    if row_value::<String>(&header, 0, PostgresStoreSetupErrorKind::CorruptCatalog)?
        != DatabaseRole::Migrator.as_str()
        || row_value::<Option<String>>(&header, 1, PostgresStoreSetupErrorKind::CorruptCatalog)?
            .as_deref()
            != Some("LATTICE_WRITER_LEASE_SCHEMA_V1")
    {
        return Err(catalog_error());
    }

    for (query, expected, error_kind) in WRITER_LEASE_V1_CATALOG_PROFILES {
        if catalog_signature(client, query, error_kind)? != expected {
            return Err(PostgresStoreSetupError::new(error_kind));
        }
    }

    let closure = client
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
                AND pg_catalog.has_table_privilege(roles.rolname,c.oid, \
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
        .map_err(|error| {
            map_postgres_error(&error, PostgresStoreSetupErrorKind::CorruptCatalog)
        })?;
    for (index, expected) in [12_i64, 0, 0, 0, 0, 0, 0, 0].into_iter().enumerate() {
        if row_value::<i64>(&closure, index, PostgresStoreSetupErrorKind::CorruptCatalog)?
            != expected
        {
            return Err(catalog_error());
        }
    }
    for index in 8..=10 {
        if row_value::<i64>(
            &closure,
            index,
            PostgresStoreSetupErrorKind::PermissionDenied,
        )? != 0
        {
            return Err(permission_error());
        }
    }
    if row_value::<bool>(&closure, 11, PostgresStoreSetupErrorKind::PermissionDenied)? {
        return Err(permission_error());
    }
    Ok(())
}

fn verify_catalog_signatures<C: GenericClient>(
    client: &mut C,
    profile: CatalogProfile,
) -> Result<(), PostgresStoreSetupError> {
    let expected = match profile {
        CatalogProfile::V1 => [
            V1_EXPECTED_RELATION_SIGNATURE,
            V1_EXPECTED_COLUMN_SIGNATURE,
            V1_EXPECTED_CONSTRAINT_SIGNATURE,
            V1_EXPECTED_INDEX_SIGNATURE,
        ],
        CatalogProfile::V2 => [
            V2_EXPECTED_RELATION_SIGNATURE,
            V2_EXPECTED_COLUMN_SIGNATURE,
            V2_EXPECTED_CONSTRAINT_SIGNATURE,
            V2_EXPECTED_INDEX_SIGNATURE,
        ],
        CatalogProfile::V3 => [
            V3_EXPECTED_RELATION_SIGNATURE,
            V3_EXPECTED_COLUMN_SIGNATURE,
            V3_EXPECTED_CONSTRAINT_SIGNATURE,
            V3_EXPECTED_INDEX_SIGNATURE,
        ],
        CatalogProfile::V3CodebaseMemoryV2 | CatalogProfile::V3CodebaseMemoryV2WriterLeaseV1 => [
            V3_CODEBASE_MEMORY_V2_EXPECTED_RELATION_SIGNATURE,
            V3_CODEBASE_MEMORY_V2_EXPECTED_COLUMN_SIGNATURE,
            V3_CODEBASE_MEMORY_V2_EXPECTED_CONSTRAINT_SIGNATURE,
            V3_CODEBASE_MEMORY_V2_EXPECTED_INDEX_SIGNATURE,
        ],
        CatalogProfile::V4 => [
            V4_EXPECTED_RELATION_SIGNATURE,
            V4_EXPECTED_COLUMN_SIGNATURE,
            V4_EXPECTED_CONSTRAINT_SIGNATURE,
            V4_EXPECTED_INDEX_SIGNATURE,
        ],
        CatalogProfile::PreSchema => return Err(catalog_error()),
    };
    for (query, expected) in [
        RELATION_SIGNATURE_SQL,
        COLUMN_SIGNATURE_SQL,
        CONSTRAINT_SIGNATURE_SQL,
        INDEX_SIGNATURE_SQL,
    ]
    .into_iter()
    .zip(expected)
    {
        let actual = catalog_signature(client, query, PostgresStoreSetupErrorKind::CorruptCatalog)?;
        if actual != expected {
            return Err(catalog_error());
        }
    }
    Ok(())
}

#[allow(clippy::too_many_lines)]
fn verify_schema_headers<C: GenericClient>(
    client: &mut C,
    profile: CatalogProfile,
) -> Result<(), PostgresStoreSetupError> {
    let schema_rows = client
        .query(
            "SELECT n.nspname, r.rolname, obj_description(n.oid, 'pg_namespace') \
             FROM pg_namespace n JOIN pg_roles r ON r.oid = n.nspowner \
             WHERE n.nspname IN ('control', 'memory', 'readmodel') ORDER BY n.nspname",
            &[],
        )
        .map_err(|error| map_postgres_error(&error, PostgresStoreSetupErrorKind::CorruptCatalog))?;
    if schema_rows.len() != CONTROL_SCHEMAS.len() {
        return Err(catalog_error());
    }
    let suffix = match profile {
        CatalogProfile::V1 => "V1",
        CatalogProfile::V2 => "V2",
        CatalogProfile::V3
        | CatalogProfile::V3CodebaseMemoryV2
        | CatalogProfile::V3CodebaseMemoryV2WriterLeaseV1 => "V3",
        CatalogProfile::V4 => "V4",
        CatalogProfile::PreSchema => return Err(catalog_error()),
    };
    let expected_comments = [
        ("control", format!("LATTICE_DEVOS_CONTROL_SCHEMA_{suffix}")),
        ("memory", format!("LATTICE_DEVOS_MEMORY_SCHEMA_{suffix}")),
        (
            "readmodel",
            format!("LATTICE_DEVOS_READMODEL_SCHEMA_{suffix}"),
        ),
    ];
    for (row, (expected_name, expected_comment)) in schema_rows.iter().zip(expected_comments) {
        let name = row_value::<String>(row, 0, PostgresStoreSetupErrorKind::CorruptCatalog)?;
        let owner = row_value::<String>(row, 1, PostgresStoreSetupErrorKind::CorruptCatalog)?;
        let comment =
            row_value::<Option<String>>(row, 2, PostgresStoreSetupErrorKind::CorruptCatalog)?;
        if name != expected_name
            || owner != DatabaseRole::Migrator.as_str()
            || comment.as_deref() != Some(expected_comment.as_str())
        {
            return Err(catalog_error());
        }
    }
    let tables = string_set(
        client,
        "SELECT c.relname FROM pg_class c \
         JOIN pg_namespace n ON n.oid = c.relnamespace \
         WHERE n.nspname = 'control' AND c.relkind = 'r' ORDER BY c.relname",
    )?;
    let expected_tables: BTreeSet<String> = match profile {
        CatalogProfile::V1 | CatalogProfile::V2 => {
            CONTROL_TABLES.into_iter().map(str::to_owned).collect()
        }
        CatalogProfile::V3
        | CatalogProfile::V3CodebaseMemoryV2
        | CatalogProfile::V3CodebaseMemoryV2WriterLeaseV1 => {
            V3_CONTROL_TABLES.into_iter().map(str::to_owned).collect()
        }
        CatalogProfile::V4 => V4_CONTROL_TABLES.into_iter().map(str::to_owned).collect(),
        CatalogProfile::PreSchema => return Err(catalog_error()),
    };
    if tables != expected_tables {
        return Err(catalog_error());
    }
    let constraints = string_set(
        client,
        "SELECT con.conname FROM pg_constraint con \
         JOIN pg_namespace n ON n.oid = con.connamespace \
         WHERE n.nspname = 'control' ORDER BY con.conname",
    )?;
    let expected_constraints: BTreeSet<String> = match profile {
        CatalogProfile::V1 => V1_CONTROL_CONSTRAINTS
            .into_iter()
            .map(str::to_owned)
            .collect(),
        CatalogProfile::V2 => V2_CONTROL_CONSTRAINTS
            .into_iter()
            .map(str::to_owned)
            .collect(),
        CatalogProfile::V3
        | CatalogProfile::V3CodebaseMemoryV2
        | CatalogProfile::V3CodebaseMemoryV2WriterLeaseV1 => V2_CONTROL_CONSTRAINTS
            .into_iter()
            .chain(TASK_LEDGER_CONTROL_CONSTRAINTS)
            .map(str::to_owned)
            .collect(),
        CatalogProfile::V4 => V2_CONTROL_CONSTRAINTS
            .into_iter()
            .chain(TASK_LEDGER_CONTROL_CONSTRAINTS)
            .chain(PROJECT_REGISTRY_CONTROL_CONSTRAINTS)
            .map(str::to_owned)
            .collect(),
        CatalogProfile::PreSchema => return Err(catalog_error()),
    };
    if constraints != expected_constraints {
        return Err(catalog_error());
    }
    verify_owned_type_closure(client, profile)?;
    Ok(())
}

fn verify_owned_type_closure<C: GenericClient>(
    client: &mut C,
    profile: CatalogProfile,
) -> Result<(), PostgresStoreSetupError> {
    let rows = client
        .query(TYPE_SIGNATURE_SQL, &[])
        .map_err(|error| map_postgres_error(&error, PostgresStoreSetupErrorKind::CorruptCatalog))?;
    let mut actual = BTreeSet::new();
    for row in &rows {
        actual.insert((
            row_value::<String>(row, 0, PostgresStoreSetupErrorKind::CorruptCatalog)?,
            row_value::<String>(row, 1, PostgresStoreSetupErrorKind::CorruptCatalog)?,
            row_value::<String>(row, 2, PostgresStoreSetupErrorKind::CorruptCatalog)?,
            row_value::<bool>(row, 3, PostgresStoreSetupErrorKind::CorruptCatalog)?,
            row_value::<String>(row, 4, PostgresStoreSetupErrorKind::CorruptCatalog)?,
            row_value::<String>(row, 5, PostgresStoreSetupErrorKind::CorruptCatalog)?,
            row_value::<String>(row, 6, PostgresStoreSetupErrorKind::CorruptCatalog)?,
            row_value::<String>(row, 7, PostgresStoreSetupErrorKind::CorruptCatalog)?,
        ));
    }

    let mut expected = BTreeSet::new();
    let expected_tables: Vec<(&str, &str)> = match profile {
        CatalogProfile::V1 | CatalogProfile::V2 => CONTROL_TABLES
            .into_iter()
            .map(|table| ("control", table))
            .collect(),
        CatalogProfile::V3 => V3_CONTROL_TABLES
            .into_iter()
            .map(|table| ("control", table))
            .collect(),
        CatalogProfile::V3CodebaseMemoryV2 => V3_CONTROL_TABLES
            .into_iter()
            .map(|table| ("control", table))
            .chain(
                CODEBASE_MEMORY_V2_TABLES
                    .into_iter()
                    .map(|table| ("memory", table)),
            )
            .collect(),
        CatalogProfile::V3CodebaseMemoryV2WriterLeaseV1 => V3_CONTROL_TABLES
            .into_iter()
            .map(|table| ("control", table))
            .chain(
                CODEBASE_MEMORY_V2_TABLES
                    .into_iter()
                    .map(|table| ("memory", table)),
            )
            .collect(),
        CatalogProfile::V4 => V4_CONTROL_TABLES
            .into_iter()
            .map(|table| ("control", table))
            .collect(),
        CatalogProfile::PreSchema => return Err(catalog_error()),
    };
    for (schema, table) in expected_tables {
        expected.insert((
            schema.to_owned(),
            table.to_owned(),
            "c".to_owned(),
            true,
            DatabaseRole::Migrator.as_str().to_owned(),
            table.to_owned(),
            "<NULL>".to_owned(),
            format!("{schema}._{table}"),
        ));
        expected.insert((
            schema.to_owned(),
            format!("_{table}"),
            "b".to_owned(),
            true,
            DatabaseRole::Migrator.as_str().to_owned(),
            "<NULL>".to_owned(),
            format!("{schema}.{table}"),
            "<NULL>".to_owned(),
        ));
    }
    if actual != expected {
        return Err(catalog_error());
    }
    Ok(())
}

#[allow(clippy::too_many_lines)]
fn verify_forbidden_schema_objects<C: GenericClient>(
    client: &mut C,
    profile: CatalogProfile,
) -> Result<(), PostgresStoreSetupError> {
    let forbidden = client
        .query_one(
            "SELECT \
             (SELECT count(*) FROM pg_proc p JOIN pg_namespace n ON n.oid = p.pronamespace \
              WHERE n.nspname IN ('control', 'memory', 'readmodel')), \
             (SELECT count(*) FROM pg_trigger t JOIN pg_class c ON c.oid = t.tgrelid \
              JOIN pg_namespace n ON n.oid = c.relnamespace \
              WHERE n.nspname IN ('control', 'memory', 'readmodel') AND NOT t.tgisinternal), \
             (SELECT count(*) FROM pg_rewrite w JOIN pg_class c ON c.oid = w.ev_class \
              JOIN pg_namespace n ON n.oid = c.relnamespace \
              WHERE n.nspname IN ('control', 'memory', 'readmodel') \
                AND w.rulename <> '_RETURN'), \
             (SELECT count(*) FROM pg_policy p JOIN pg_class c ON c.oid = p.polrelid \
              JOIN pg_namespace n ON n.oid = c.relnamespace \
              WHERE n.nspname IN ('control', 'memory', 'readmodel')), \
             (SELECT count(*) FROM pg_type t JOIN pg_namespace n ON n.oid = t.typnamespace \
              WHERE n.nspname IN ('control', 'memory', 'readmodel') \
                AND (NOT t.typisdefined OR t.typtype IN ('d', 'e', 'p', 'r', 'm'))), \
             (SELECT count(*) FROM pg_event_trigger), \
             (SELECT count(*) FROM pg_trigger t \
              JOIN pg_constraint con ON con.oid = t.tgconstraint \
              JOIN pg_namespace n ON n.oid = con.connamespace \
              WHERE n.nspname = 'control' AND t.tgisinternal \
                AND t.tgenabled = 'O' \
                AND con.conname = 'terminal_transactions_scope_head_fk'), \
              (SELECT count(*) FROM pg_trigger t JOIN pg_class c ON c.oid = t.tgrelid \
               JOIN pg_namespace n ON n.oid = c.relnamespace \
               WHERE n.nspname IN ('control', 'memory', 'readmodel') \
                 AND t.tgisinternal AND t.tgenabled = 'O'), \
              (SELECT count(*) FROM pg_inherits i \
               JOIN pg_class parent ON parent.oid = i.inhparent \
               JOIN pg_namespace parent_ns ON parent_ns.oid = parent.relnamespace \
               JOIN pg_class child ON child.oid = i.inhrelid \
               JOIN pg_namespace child_ns ON child_ns.oid = child.relnamespace \
               WHERE parent_ns.nspname IN ('control', 'memory', 'readmodel') \
                  OR child_ns.nspname IN ('control', 'memory', 'readmodel')), \
              (SELECT count(*) FROM pg_class c \
               JOIN pg_namespace n ON n.oid = c.relnamespace \
               WHERE n.nspname IN ('control', 'memory', 'readmodel') \
                 AND (c.relhassubclass OR c.relispartition))",
            &[],
        )
        .map_err(|error| map_postgres_error(&error, PostgresStoreSetupErrorKind::CorruptCatalog))?;
    let expected_functions = match profile {
        CatalogProfile::V1 | CatalogProfile::PreSchema => 0,
        CatalogProfile::V2 => 3,
        CatalogProfile::V3 => 11,
        CatalogProfile::V3CodebaseMemoryV2 | CatalogProfile::V3CodebaseMemoryV2WriterLeaseV1 => 18,
        CatalogProfile::V4 => 28,
    };
    if row_value::<i64>(&forbidden, 0, PostgresStoreSetupErrorKind::CorruptCatalog)?
        != expected_functions
    {
        return Err(catalog_error());
    }
    for index in 1..6 {
        if row_value::<i64>(
            &forbidden,
            index,
            PostgresStoreSetupErrorKind::CorruptCatalog,
        )? != 0
        {
            return Err(catalog_error());
        }
    }
    let expected_scope_head_triggers = match profile {
        CatalogProfile::V1 => 4,
        CatalogProfile::V2
        | CatalogProfile::V3
        | CatalogProfile::V3CodebaseMemoryV2
        | CatalogProfile::V3CodebaseMemoryV2WriterLeaseV1
        | CatalogProfile::V4
        | CatalogProfile::PreSchema => 0,
    };
    let expected_internal_triggers = match profile {
        CatalogProfile::V1 => 4,
        CatalogProfile::V2 | CatalogProfile::PreSchema => 0,
        CatalogProfile::V3 => 20,
        CatalogProfile::V3CodebaseMemoryV2 | CatalogProfile::V3CodebaseMemoryV2WriterLeaseV1 => 44,
        CatalogProfile::V4 => 40,
    };
    if row_value::<i64>(&forbidden, 6, PostgresStoreSetupErrorKind::CorruptCatalog)?
        != expected_scope_head_triggers
        || row_value::<i64>(&forbidden, 7, PostgresStoreSetupErrorKind::CorruptCatalog)?
            != expected_internal_triggers
        || row_value::<i64>(&forbidden, 8, PostgresStoreSetupErrorKind::CorruptCatalog)? != 0
        || row_value::<i64>(&forbidden, 9, PostgresStoreSetupErrorKind::CorruptCatalog)? != 0
    {
        return Err(catalog_error());
    }
    Ok(())
}

#[allow(clippy::too_many_lines)]
fn verify_owned_function_boundary<C: GenericClient>(
    client: &mut C,
    profile: CatalogProfile,
) -> Result<(), PostgresStoreSetupError> {
    let signature = catalog_signature(
        client,
        FUNCTION_SIGNATURE_SQL,
        PostgresStoreSetupErrorKind::CorruptCatalog,
    )?;
    let expected_signature = match profile {
        CatalogProfile::V2 => V2_EXPECTED_FUNCTION_SIGNATURE,
        CatalogProfile::V3 => V3_EXPECTED_FUNCTION_SIGNATURE,
        CatalogProfile::V3CodebaseMemoryV2 | CatalogProfile::V3CodebaseMemoryV2WriterLeaseV1 => {
            V3_CODEBASE_MEMORY_V2_EXPECTED_FUNCTION_SIGNATURE
        }
        CatalogProfile::V4 => V4_EXPECTED_FUNCTION_SIGNATURE,
        CatalogProfile::V1 | CatalogProfile::PreSchema => return Err(catalog_error()),
    };
    if signature != expected_signature {
        return Err(catalog_error());
    }

    let expected_identities: BTreeSet<String> = match profile {
        CatalogProfile::V2 => [
            STORE_PREPARE_V2_IDENTITY,
            STORE_FINALIZE_V2_IDENTITY,
            STORE_CURRENT_HEAD_V2_IDENTITY,
        ]
        .into_iter()
        .map(str::to_owned)
        .collect(),
        CatalogProfile::V3 => [
            STORE_PREPARE_V2_IDENTITY,
            STORE_FINALIZE_V2_IDENTITY,
            STORE_CURRENT_HEAD_V2_IDENTITY,
            STORE_PREPARE_V3_IDENTITY,
            STORE_FINALIZE_V3_IDENTITY,
            STORE_CURRENT_HEAD_V3_IDENTITY,
            TASK_LEDGER_PREPARE_V1_IDENTITY,
            TASK_LEDGER_READ_HEAD_V1_IDENTITY,
            TASK_LEDGER_READ_EVENTS_V1_IDENTITY,
            TASK_LEDGER_READ_COMMANDS_V1_IDENTITY,
            TASK_LEDGER_FINALIZE_V1_IDENTITY,
        ]
        .into_iter()
        .map(str::to_owned)
        .collect(),
        CatalogProfile::V3CodebaseMemoryV2 | CatalogProfile::V3CodebaseMemoryV2WriterLeaseV1 => [
            STORE_PREPARE_V2_IDENTITY,
            STORE_FINALIZE_V2_IDENTITY,
            STORE_CURRENT_HEAD_V2_IDENTITY,
            STORE_PREPARE_V3_IDENTITY,
            STORE_FINALIZE_V3_IDENTITY,
            STORE_CURRENT_HEAD_V3_IDENTITY,
            TASK_LEDGER_PREPARE_V1_IDENTITY,
            TASK_LEDGER_READ_HEAD_V1_IDENTITY,
            TASK_LEDGER_READ_EVENTS_V1_IDENTITY,
            TASK_LEDGER_READ_COMMANDS_V1_IDENTITY,
            TASK_LEDGER_FINALIZE_V1_IDENTITY,
            CODEBASE_MEMORY_LOAD_REFLECTION_V2_IDENTITY,
            CODEBASE_MEMORY_LOAD_RECEIPT_V1_IDENTITY,
            CODEBASE_MEMORY_PERSIST_ANALYSIS_V1_IDENTITY,
            CODEBASE_MEMORY_PERSIST_REFLECTION_V2_IDENTITY,
            CODEBASE_MEMORY_PERSIST_RETRIEVAL_V1_IDENTITY,
            OPENCLAW_GATEWAY_FINALIZE_TERMINAL_V1_IDENTITY,
            OPENCLAW_GATEWAY_RECONCILE_AND_CLAIM_V1_IDENTITY,
        ]
        .into_iter()
        .map(str::to_owned)
        .collect(),
        CatalogProfile::V4 => V3_CONTROL_FUNCTION_IDENTITIES
            .into_iter()
            .chain(V4_RUNTIME_FUNCTION_IDENTITIES)
            .map(str::to_owned)
            .collect(),
        CatalogProfile::V1 | CatalogProfile::PreSchema => return Err(catalog_error()),
    };
    let rows = client
        .query(
            "SELECT n.nspname || '.' || p.proname || '(' || \
                    replace(pg_catalog.oidvectortypes(p.proargtypes), ' ', '') || ')', \
                    pg_get_userbyid(p.proowner), p.prosecdef, p.proleakproof, \
                    COALESCE(array_to_string(p.proconfig, ','), '<NULL>') \
             FROM pg_proc p JOIN pg_namespace n ON n.oid = p.pronamespace \
             WHERE n.nspname IN ('control', 'memory', 'readmodel') \
             ORDER BY n.nspname, p.proname, pg_catalog.oidvectortypes(p.proargtypes)",
            &[],
        )
        .map_err(|error| map_postgres_error(&error, PostgresStoreSetupErrorKind::CorruptCatalog))?;
    let mut actual_identities = BTreeSet::new();
    for row in &rows {
        let identity = row_value::<String>(row, 0, PostgresStoreSetupErrorKind::CorruptCatalog)?;
        let expected_proconfig = if [
            STORE_PREPARE_V3_IDENTITY,
            STORE_FINALIZE_V3_IDENTITY,
            STORE_CURRENT_HEAD_V3_IDENTITY,
            TASK_LEDGER_PREPARE_V1_IDENTITY,
            TASK_LEDGER_READ_HEAD_V1_IDENTITY,
            TASK_LEDGER_READ_EVENTS_V1_IDENTITY,
            TASK_LEDGER_READ_COMMANDS_V1_IDENTITY,
            TASK_LEDGER_FINALIZE_V1_IDENTITY,
            CODEBASE_MEMORY_LOAD_REFLECTION_V2_IDENTITY,
            CODEBASE_MEMORY_LOAD_RECEIPT_V1_IDENTITY,
            CODEBASE_MEMORY_PERSIST_ANALYSIS_V1_IDENTITY,
            CODEBASE_MEMORY_PERSIST_REFLECTION_V2_IDENTITY,
            CODEBASE_MEMORY_PERSIST_RETRIEVAL_V1_IDENTITY,
            OPENCLAW_GATEWAY_FINALIZE_TERMINAL_V1_IDENTITY,
            OPENCLAW_GATEWAY_RECONCILE_AND_CLAIM_V1_IDENTITY,
        ]
        .contains(&identity.as_str())
            || V4_RUNTIME_FUNCTION_IDENTITIES.contains(&identity.as_str())
        {
            "search_path=pg_catalog,row_security=on,lock_timeout=5s,statement_timeout=30s"
        } else {
            "search_path=pg_catalog,row_security=on"
        };
        if row_value::<String>(row, 1, PostgresStoreSetupErrorKind::CorruptCatalog)?
            != DatabaseRole::Migrator.as_str()
            || !row_value::<bool>(row, 2, PostgresStoreSetupErrorKind::CorruptCatalog)?
            || row_value::<bool>(row, 3, PostgresStoreSetupErrorKind::CorruptCatalog)?
            || row_value::<String>(row, 4, PostgresStoreSetupErrorKind::CorruptCatalog)?
                != expected_proconfig
        {
            return Err(catalog_error());
        }
        actual_identities.insert(identity);
    }
    if actual_identities != expected_identities {
        return Err(catalog_error());
    }
    Ok(())
}

fn verify_forbidden_namespace_objects<C: GenericClient>(
    client: &mut C,
) -> Result<(), PostgresStoreSetupError> {
    let row = client
        .query_one(
            "SELECT \
             (SELECT count(*) FROM pg_collation c JOIN pg_namespace n ON n.oid = c.collnamespace \
              WHERE n.nspname IN ('control', 'memory', 'readmodel')), \
             (SELECT count(*) FROM pg_conversion c JOIN pg_namespace n ON n.oid = c.connamespace \
              WHERE n.nspname IN ('control', 'memory', 'readmodel')), \
             (SELECT count(*) FROM pg_statistic_ext s JOIN pg_namespace n ON n.oid = s.stxnamespace \
              WHERE n.nspname IN ('control', 'memory', 'readmodel')), \
             (SELECT count(*) FROM pg_ts_config c JOIN pg_namespace n ON n.oid = c.cfgnamespace \
              WHERE n.nspname IN ('control', 'memory', 'readmodel')), \
             (SELECT count(*) FROM pg_ts_dict d JOIN pg_namespace n ON n.oid = d.dictnamespace \
              WHERE n.nspname IN ('control', 'memory', 'readmodel')), \
             (SELECT count(*) FROM pg_ts_parser p JOIN pg_namespace n ON n.oid = p.prsnamespace \
              WHERE n.nspname IN ('control', 'memory', 'readmodel')), \
             (SELECT count(*) FROM pg_ts_template t JOIN pg_namespace n ON n.oid = t.tmplnamespace \
              WHERE n.nspname IN ('control', 'memory', 'readmodel')), \
             (SELECT count(*) FROM pg_operator o JOIN pg_namespace n ON n.oid = o.oprnamespace \
              WHERE n.nspname IN ('control', 'memory', 'readmodel')), \
             (SELECT count(*) FROM pg_opclass o JOIN pg_namespace n ON n.oid = o.opcnamespace \
              WHERE n.nspname IN ('control', 'memory', 'readmodel')), \
             (SELECT count(*) FROM pg_opfamily o JOIN pg_namespace n ON n.oid = o.opfnamespace \
              WHERE n.nspname IN ('control', 'memory', 'readmodel'))",
            &[],
        )
        .map_err(|error| map_postgres_error(&error, PostgresStoreSetupErrorKind::CorruptCatalog))?;
    for index in 0..10 {
        if row_value::<i64>(&row, index, PostgresStoreSetupErrorKind::CorruptCatalog)? != 0 {
            return Err(catalog_error());
        }
    }
    Ok(())
}

fn verify_roles_and_grants<C: GenericClient>(
    client: &mut C,
    profile: CatalogProfile,
) -> Result<(), PostgresStoreSetupError> {
    verify_role_and_database_boundary(client, profile)?;
    let expected_schema_acl = match profile {
        CatalogProfile::V3CodebaseMemoryV2 | CatalogProfile::V3CodebaseMemoryV2WriterLeaseV1 => {
            V3_CODEBASE_MEMORY_V2_EXPECTED_SCHEMA_ACL_SIGNATURE
        }
        CatalogProfile::PreSchema
        | CatalogProfile::V1
        | CatalogProfile::V2
        | CatalogProfile::V3
        | CatalogProfile::V4 => EXPECTED_SCHEMA_ACL_SIGNATURE,
    };
    let expected_table_acl = match profile {
        CatalogProfile::PreSchema | CatalogProfile::V1 | CatalogProfile::V2 => {
            EXPECTED_TABLE_ACL_SIGNATURE
        }
        CatalogProfile::V3 => V3_EXPECTED_TABLE_ACL_SIGNATURE,
        CatalogProfile::V3CodebaseMemoryV2 | CatalogProfile::V3CodebaseMemoryV2WriterLeaseV1 => {
            V3_CODEBASE_MEMORY_V2_EXPECTED_TABLE_ACL_SIGNATURE
        }
        CatalogProfile::V4 => V4_EXPECTED_TABLE_ACL_SIGNATURE,
    };
    for (query, expected) in [
        (SCHEMA_ACL_SIGNATURE_SQL, expected_schema_acl),
        (TABLE_ACL_SIGNATURE_SQL, expected_table_acl),
        (DEFAULT_ACL_SIGNATURE_SQL, EXPECTED_DEFAULT_ACL_SIGNATURE),
    ] {
        let actual =
            catalog_signature(client, query, PostgresStoreSetupErrorKind::PermissionDenied)?;
        if actual != expected {
            return Err(permission_error());
        }
    }
    if matches!(
        profile,
        CatalogProfile::V2
            | CatalogProfile::V3
            | CatalogProfile::V3CodebaseMemoryV2
            | CatalogProfile::V3CodebaseMemoryV2WriterLeaseV1
            | CatalogProfile::V4
    ) {
        verify_owned_function_acl(client, profile)?;
    }
    for role in [
        DatabaseRole::Runtime,
        DatabaseRole::Guardian,
        DatabaseRole::ReadOnly,
    ] {
        verify_nonwriter_capabilities(client, role, profile)?;
    }
    verify_effective_default_privileges(client)
}

#[allow(clippy::too_many_lines)]
fn verify_owned_function_acl<C: GenericClient>(
    client: &mut C,
    profile: CatalogProfile,
) -> Result<(), PostgresStoreSetupError> {
    let signature = catalog_signature(
        client,
        FUNCTION_ACL_SIGNATURE_SQL,
        PostgresStoreSetupErrorKind::PermissionDenied,
    )?;
    let expected_signature = match profile {
        CatalogProfile::V2 => V2_EXPECTED_FUNCTION_ACL_SIGNATURE,
        CatalogProfile::V3 => V3_EXPECTED_FUNCTION_ACL_SIGNATURE,
        CatalogProfile::V3CodebaseMemoryV2 | CatalogProfile::V3CodebaseMemoryV2WriterLeaseV1 => {
            V3_CODEBASE_MEMORY_V2_EXPECTED_FUNCTION_ACL_SIGNATURE
        }
        CatalogProfile::V4 => V4_EXPECTED_FUNCTION_ACL_SIGNATURE,
        CatalogProfile::V1 | CatalogProfile::PreSchema => return Err(permission_error()),
    };
    if signature != expected_signature {
        return Err(permission_error());
    }

    let runtime_identities: BTreeSet<String> = match profile {
        CatalogProfile::V2 => [
            STORE_PREPARE_V2_IDENTITY,
            STORE_FINALIZE_V2_IDENTITY,
            STORE_CURRENT_HEAD_V2_IDENTITY,
        ]
        .into_iter()
        .map(str::to_owned)
        .collect(),
        CatalogProfile::V3 => [
            STORE_PREPARE_V3_IDENTITY,
            STORE_FINALIZE_V3_IDENTITY,
            STORE_CURRENT_HEAD_V3_IDENTITY,
            TASK_LEDGER_PREPARE_V1_IDENTITY,
            TASK_LEDGER_READ_HEAD_V1_IDENTITY,
            TASK_LEDGER_READ_EVENTS_V1_IDENTITY,
            TASK_LEDGER_READ_COMMANDS_V1_IDENTITY,
            TASK_LEDGER_FINALIZE_V1_IDENTITY,
        ]
        .into_iter()
        .map(str::to_owned)
        .collect(),
        CatalogProfile::V3CodebaseMemoryV2 | CatalogProfile::V3CodebaseMemoryV2WriterLeaseV1 => [
            STORE_PREPARE_V3_IDENTITY,
            STORE_FINALIZE_V3_IDENTITY,
            STORE_CURRENT_HEAD_V3_IDENTITY,
            TASK_LEDGER_PREPARE_V1_IDENTITY,
            TASK_LEDGER_READ_HEAD_V1_IDENTITY,
            TASK_LEDGER_READ_EVENTS_V1_IDENTITY,
            TASK_LEDGER_READ_COMMANDS_V1_IDENTITY,
            TASK_LEDGER_FINALIZE_V1_IDENTITY,
            CODEBASE_MEMORY_LOAD_REFLECTION_V2_IDENTITY,
            CODEBASE_MEMORY_LOAD_RECEIPT_V1_IDENTITY,
            CODEBASE_MEMORY_PERSIST_ANALYSIS_V1_IDENTITY,
            CODEBASE_MEMORY_PERSIST_REFLECTION_V2_IDENTITY,
            CODEBASE_MEMORY_PERSIST_RETRIEVAL_V1_IDENTITY,
            OPENCLAW_GATEWAY_FINALIZE_TERMINAL_V1_IDENTITY,
            OPENCLAW_GATEWAY_RECONCILE_AND_CLAIM_V1_IDENTITY,
        ]
        .into_iter()
        .map(str::to_owned)
        .collect(),
        CatalogProfile::V4 => V4_RUNTIME_FUNCTION_IDENTITIES
            .into_iter()
            .map(str::to_owned)
            .collect(),
        CatalogProfile::V1 | CatalogProfile::PreSchema => return Err(permission_error()),
    };
    let rows = client
        .query(
            "SELECT n.nspname || '.' || p.proname || '(' || \
                    replace(pg_catalog.oidvectortypes(p.proargtypes), ' ', '') || ')', \
                    has_function_privilege('public', p.oid, 'EXECUTE'), \
                    has_function_privilege('lattice_runtime', p.oid, 'EXECUTE'), \
                    has_function_privilege('lattice_guardian', p.oid, 'EXECUTE'), \
                    has_function_privilege('lattice_readonly', p.oid, 'EXECUTE'), \
                    has_function_privilege('lattice_migrator_login', p.oid, 'EXECUTE'), \
                    has_function_privilege('lattice_runtime_login', p.oid, 'EXECUTE'), \
                    has_function_privilege('lattice_guardian_login', p.oid, 'EXECUTE'), \
                    has_function_privilege('lattice_readonly_login', p.oid, 'EXECUTE'), \
                    (SELECT count(*) FROM aclexplode( \
                        COALESCE(p.proacl, acldefault('f', p.proowner))) acl \
                     LEFT JOIN pg_roles grantee ON grantee.oid = acl.grantee \
                     JOIN pg_roles grantor ON grantor.oid = acl.grantor \
                     WHERE acl.grantee = 0 \
                        OR grantee.rolname NOT IN ('lattice_migrator', 'lattice_runtime') \
                        OR acl.privilege_type <> 'EXECUTE' OR acl.is_grantable \
                        OR grantor.rolname <> 'lattice_migrator'), \
                    (SELECT count(*) FROM aclexplode( \
                        COALESCE(p.proacl, acldefault('f', p.proowner))) acl \
                     JOIN pg_roles grantee ON grantee.oid = acl.grantee \
                     WHERE grantee.rolname = 'lattice_runtime' \
                       AND acl.privilege_type = 'EXECUTE' AND NOT acl.is_grantable) \
             FROM pg_proc p JOIN pg_namespace n ON n.oid = p.pronamespace \
             WHERE n.nspname IN ('control', 'memory', 'readmodel') \
             ORDER BY n.nspname, p.proname, pg_catalog.oidvectortypes(p.proargtypes)",
            &[],
        )
        .map_err(|error| {
            map_postgres_error(&error, PostgresStoreSetupErrorKind::PermissionDenied)
        })?;
    let mut observed_runtime = BTreeSet::new();
    for row in &rows {
        let identity = row_value::<String>(row, 0, PostgresStoreSetupErrorKind::PermissionDenied)?;
        let runtime_expected = runtime_identities.contains(&identity);
        if row_value::<bool>(row, 1, PostgresStoreSetupErrorKind::PermissionDenied)?
            || row_value::<bool>(row, 2, PostgresStoreSetupErrorKind::PermissionDenied)?
                != runtime_expected
            || (3..=8).any(|index| {
                row_value::<bool>(row, index, PostgresStoreSetupErrorKind::PermissionDenied)
                    .unwrap_or(true)
            })
            || row_value::<i64>(row, 9, PostgresStoreSetupErrorKind::PermissionDenied)? != 0
            || row_value::<i64>(row, 10, PostgresStoreSetupErrorKind::PermissionDenied)?
                != i64::from(runtime_expected)
        {
            return Err(permission_error());
        }
        if runtime_expected {
            observed_runtime.insert(identity);
        }
    }
    if observed_runtime != runtime_identities {
        return Err(permission_error());
    }
    Ok(())
}

fn verify_role_and_database_boundary<C: GenericClient>(
    client: &mut C,
    profile: CatalogProfile,
) -> Result<(), PostgresStoreSetupError> {
    let role_signature = catalog_signature(
        client,
        ROLE_SIGNATURE_SQL,
        PostgresStoreSetupErrorKind::PermissionDenied,
    )?;
    let database_acl_signature = catalog_signature(
        client,
        DATABASE_ACL_SIGNATURE_SQL,
        PostgresStoreSetupErrorKind::PermissionDenied,
    )?;
    if role_signature != EXPECTED_ROLE_SIGNATURE
        || database_acl_signature != EXPECTED_DATABASE_ACL_SIGNATURE
    {
        return Err(permission_error());
    }

    let boundary = client
        .query_one(
            "SELECT r.rolname, d.datistemplate, d.datallowconn, d.datconnlimit, \
             (SELECT count(*) FROM pg_auth_members m \
              WHERE m.roleid IN (SELECT oid FROM pg_roles WHERE rolname LIKE 'lattice\\_%' ESCAPE '\\') \
                 OR m.member IN (SELECT oid FROM pg_roles WHERE rolname LIKE 'lattice\\_%' ESCAPE '\\')), \
             (SELECT count(*) FROM pg_roles \
               WHERE rolname LIKE 'lattice\\_%' ESCAPE '\\' \
                 AND rolname NOT IN ('lattice_migrator', 'lattice_runtime', \
                                     'lattice_guardian', 'lattice_readonly', \
                                     'lattice_migrator_login', 'lattice_runtime_login', \
                                     'lattice_guardian_login', 'lattice_readonly_login')), \
             (SELECT count(*) FROM pg_db_role_setting s \
              WHERE s.setrole IN (SELECT oid FROM pg_roles \
                   WHERE rolname IN ('lattice_migrator', 'lattice_runtime', \
                                     'lattice_guardian', 'lattice_readonly', \
                                     'lattice_migrator_login', 'lattice_runtime_login', \
                                     'lattice_guardian_login', 'lattice_readonly_login'))), \
             (SELECT count(*) FROM pg_proc p \
              JOIN pg_namespace n ON n.oid = p.pronamespace \
              WHERE p.prosecdef \
                AND n.nspname !~ '^pg_' AND n.nspname <> 'information_schema' \
                AND (has_function_privilege('lattice_runtime', p.oid, 'EXECUTE') \
                  OR has_function_privilege('lattice_guardian', p.oid, 'EXECUTE') \
                  OR has_function_privilege('lattice_readonly', p.oid, 'EXECUTE'))), \
             has_database_privilege('public', current_database(), 'CONNECT'), \
             has_database_privilege('public', current_database(), 'CREATE'), \
             has_database_privilege('public', current_database(), 'TEMPORARY'), \
             has_database_privilege('lattice_migrator', current_database(), 'CONNECT'), \
             has_database_privilege('lattice_migrator', current_database(), 'CREATE'), \
             has_database_privilege('lattice_migrator', current_database(), 'TEMPORARY') \
             FROM pg_database d JOIN pg_roles r ON r.oid = d.datdba \
             WHERE d.datname = current_database()",
            &[],
        )
        .map_err(|error| {
            map_postgres_error(&error, PostgresStoreSetupErrorKind::PermissionDenied)
        })?;
    let owner = row_value::<String>(&boundary, 0, PostgresStoreSetupErrorKind::PermissionDenied)?;
    let is_template =
        row_value::<bool>(&boundary, 1, PostgresStoreSetupErrorKind::PermissionDenied)?;
    let allows_connections =
        row_value::<bool>(&boundary, 2, PostgresStoreSetupErrorKind::PermissionDenied)?;
    let connection_limit =
        row_value::<i32>(&boundary, 3, PostgresStoreSetupErrorKind::PermissionDenied)?;
    let memberships =
        row_value::<i64>(&boundary, 4, PostgresStoreSetupErrorKind::PermissionDenied)?;
    let extra_roles =
        row_value::<i64>(&boundary, 5, PostgresStoreSetupErrorKind::PermissionDenied)?;
    let role_settings =
        row_value::<i64>(&boundary, 6, PostgresStoreSetupErrorKind::PermissionDenied)?;
    let dangerous_functions =
        row_value::<i64>(&boundary, 7, PostgresStoreSetupErrorKind::PermissionDenied)?;
    let database_privileges = [
        row_value::<bool>(&boundary, 8, PostgresStoreSetupErrorKind::PermissionDenied)?,
        row_value::<bool>(&boundary, 9, PostgresStoreSetupErrorKind::PermissionDenied)?,
        row_value::<bool>(&boundary, 10, PostgresStoreSetupErrorKind::PermissionDenied)?,
        row_value::<bool>(&boundary, 11, PostgresStoreSetupErrorKind::PermissionDenied)?,
        row_value::<bool>(&boundary, 12, PostgresStoreSetupErrorKind::PermissionDenied)?,
        row_value::<bool>(&boundary, 13, PostgresStoreSetupErrorKind::PermissionDenied)?,
    ];
    let expected_dangerous_functions = match profile {
        CatalogProfile::PreSchema | CatalogProfile::V1 => 0,
        CatalogProfile::V2 => 3,
        CatalogProfile::V3 => 8,
        CatalogProfile::V3CodebaseMemoryV2 => 15,
        CatalogProfile::V3CodebaseMemoryV2WriterLeaseV1 => 22,
        CatalogProfile::V4 => 17,
    };
    if owner != DatabaseRole::Migrator.as_str()
        || is_template
        || !allows_connections
        || connection_limit != -1
        || memberships != 4
        || extra_roles != 0
        || role_settings != 0
        || dangerous_functions != expected_dangerous_functions
        || database_privileges != [false, false, false, true, true, true]
    {
        return Err(permission_error());
    }
    verify_cluster_wide_acl_closure(client, profile)?;
    verify_login_principal_closure(client)
}

fn verify_cluster_wide_acl_closure<C: GenericClient>(
    client: &mut C,
    profile: CatalogProfile,
) -> Result<(), PostgresStoreSetupError> {
    let parameter_grants = client
        .query_one(
            "SELECT count(*) FROM pg_parameter_acl p \
             CROSS JOIN LATERAL aclexplode(p.paracl) acl \
             WHERE acl.grantee = 0 \
                OR acl.grantee IN (SELECT oid FROM pg_roles \
                    WHERE rolname IN ('lattice_migrator', 'lattice_runtime', \
                        'lattice_guardian', 'lattice_readonly', \
                        'lattice_migrator_login', 'lattice_runtime_login', \
                        'lattice_guardian_login', 'lattice_readonly_login'))",
            &[],
        )
        .map_err(|error| {
            map_postgres_error(&error, PostgresStoreSetupErrorKind::PermissionDenied)
        })?;
    if row_value::<i64>(
        &parameter_grants,
        0,
        PostgresStoreSetupErrorKind::PermissionDenied,
    )? != 0
    {
        return Err(permission_error());
    }

    let public_database_grants = client
        .query_one(
            "SELECT count(*) FROM pg_database d \
             CROSS JOIN LATERAL aclexplode(COALESCE(d.datacl, acldefault('d', d.datdba))) acl \
             WHERE acl.grantee = 0",
            &[],
        )
        .map_err(|error| {
            map_postgres_error(&error, PostgresStoreSetupErrorKind::PermissionDenied)
        })?;
    if row_value::<i64>(
        &public_database_grants,
        0,
        PostgresStoreSetupErrorKind::PermissionDenied,
    )? != 0
    {
        return Err(permission_error());
    }
    let writer_lease_is_owned = matches!(profile, CatalogProfile::V3CodebaseMemoryV2WriterLeaseV1);
    verify_external_relation_principal_closure(client, writer_lease_is_owned)?;
    verify_external_function_principal_closure(client, writer_lease_is_owned)?;
    verify_pre_role_system_function_boundary(client)?;
    verify_large_object_boundary(client)
}

fn verify_external_relation_principal_closure<C: GenericClient>(
    client: &mut C,
    writer_lease_is_owned: bool,
) -> Result<(), PostgresStoreSetupError> {
    let forbidden = client
        .query_one(
            "WITH fixed_principals AS ( \
                 SELECT oid FROM pg_roles \
                 WHERE rolname IN ('lattice_migrator', 'lattice_runtime', \
                     'lattice_guardian', 'lattice_readonly', \
                     'lattice_migrator_login', 'lattice_runtime_login', \
                     'lattice_guardian_login', 'lattice_readonly_login') \
             ), external_relations AS ( \
                 SELECT c.oid, c.relowner, c.relacl \
                 FROM pg_class c \
                 JOIN pg_namespace n ON n.oid = c.relnamespace \
                 WHERE n.nspname !~ '^pg_' \
                   AND n.nspname <> 'information_schema' \
                   AND n.nspname NOT IN ('control', 'memory', 'readmodel') \
                   AND (NOT $1 OR n.nspname <> 'writer_lease') \
             ) \
             SELECT \
               (SELECT count(*) FROM external_relations c \
                WHERE c.relowner IN (SELECT oid FROM fixed_principals)), \
               (SELECT count(*) FROM external_relations c \
                CROSS JOIN LATERAL aclexplode(c.relacl) acl \
                WHERE acl.grantee = 0 \
                   OR acl.grantee IN (SELECT oid FROM fixed_principals)), \
               (SELECT count(*) FROM pg_attribute a \
                JOIN external_relations c ON c.oid = a.attrelid \
                CROSS JOIN LATERAL aclexplode(a.attacl) acl \
                WHERE acl.grantee = 0 \
                   OR acl.grantee IN (SELECT oid FROM fixed_principals))",
            &[&writer_lease_is_owned],
        )
        .map_err(|error| {
            map_postgres_error(&error, PostgresStoreSetupErrorKind::PermissionDenied)
        })?;
    for index in 0..3 {
        if row_value::<i64>(
            &forbidden,
            index,
            PostgresStoreSetupErrorKind::PermissionDenied,
        )? != 0
        {
            return Err(permission_error());
        }
    }
    Ok(())
}

fn verify_external_function_principal_closure<C: GenericClient>(
    client: &mut C,
    writer_lease_is_owned: bool,
) -> Result<(), PostgresStoreSetupError> {
    let forbidden = client
        .query_one(
            "WITH fixed_principals AS ( \
                 SELECT oid FROM pg_roles \
                 WHERE rolname IN ('lattice_migrator', 'lattice_runtime', \
                     'lattice_guardian', 'lattice_readonly', \
                     'lattice_migrator_login', 'lattice_runtime_login', \
                     'lattice_guardian_login', 'lattice_readonly_login') \
             ) \
             SELECT count(*) FROM pg_proc p \
             JOIN pg_namespace n ON n.oid = p.pronamespace \
             CROSS JOIN LATERAL aclexplode( \
                 COALESCE(p.proacl, acldefault('f', p.proowner)) \
             ) acl \
             WHERE n.nspname !~ '^pg_' \
               AND n.nspname <> 'information_schema' \
               AND n.nspname NOT IN ('control', 'memory', 'readmodel') \
               AND (NOT $1 OR n.nspname <> 'writer_lease') \
               AND (acl.grantee = 0 \
                    OR acl.grantee IN (SELECT oid FROM fixed_principals))",
            &[&writer_lease_is_owned],
        )
        .map_err(|error| {
            map_postgres_error(&error, PostgresStoreSetupErrorKind::PermissionDenied)
        })?;
    if row_value::<i64>(&forbidden, 0, PostgresStoreSetupErrorKind::PermissionDenied)? != 0 {
        return Err(permission_error());
    }
    Ok(())
}

fn verify_pre_role_system_function_boundary<C: GenericClient>(
    client: &mut C,
) -> Result<(), PostgresStoreSetupError> {
    let boundary = client
        .query_one(
            "WITH expected(signature, allowed_role) AS (VALUES \
                 ('pg_catalog.lo_creat(integer)', NULL::text), \
                 ('pg_catalog.lo_create(oid)', NULL::text), \
                 ('pg_catalog.lo_from_bytea(oid,bytea)', NULL::text), \
                 ('pg_catalog.lo_import(text)', NULL::text), \
                 ('pg_catalog.lo_import(text,oid)', NULL::text), \
                 ('pg_catalog.pg_logical_emit_message(boolean,text,text,boolean)', NULL::text), \
                 ('pg_catalog.pg_logical_emit_message(boolean,text,bytea,boolean)', NULL::text), \
                 ('pg_catalog.pg_advisory_lock(bigint)', NULL::text), \
                 ('pg_catalog.pg_advisory_lock(integer,integer)', NULL::text), \
                 ('pg_catalog.pg_advisory_lock_shared(bigint)', NULL::text), \
                 ('pg_catalog.pg_advisory_lock_shared(integer,integer)', NULL::text), \
                 ('pg_catalog.pg_try_advisory_lock(bigint)', NULL::text), \
                 ('pg_catalog.pg_try_advisory_lock(integer,integer)', NULL::text), \
                 ('pg_catalog.pg_try_advisory_lock_shared(bigint)', NULL::text), \
                 ('pg_catalog.pg_try_advisory_lock_shared(integer,integer)', NULL::text), \
                 ('pg_catalog.pg_advisory_xact_lock(bigint)', 'lattice_migrator'::text), \
                 ('pg_catalog.pg_advisory_xact_lock(integer,integer)', NULL::text), \
                 ('pg_catalog.pg_advisory_xact_lock_shared(bigint)', NULL::text), \
                 ('pg_catalog.pg_advisory_xact_lock_shared(integer,integer)', NULL::text), \
                 ('pg_catalog.pg_try_advisory_xact_lock(bigint)', NULL::text), \
                 ('pg_catalog.pg_try_advisory_xact_lock(integer,integer)', NULL::text), \
                 ('pg_catalog.pg_try_advisory_xact_lock_shared(bigint)', NULL::text), \
                 ('pg_catalog.pg_try_advisory_xact_lock_shared(integer,integer)', NULL::text), \
                 ('pg_catalog.pg_cancel_backend(integer)', NULL::text), \
                 ('pg_catalog.pg_terminate_backend(integer,bigint)', NULL::text), \
                 ('pg_catalog.pg_export_snapshot()', NULL::text), \
                 ('pg_catalog.pg_current_xact_id()', 'lattice_migrator'::text), \
                 ('pg_catalog.txid_current()', NULL::text) \
             ), resolved AS ( \
                 SELECT signature, allowed_role, to_regprocedure(signature) AS function_oid \
                 FROM expected \
             ), fixed_roles(role_name) AS (VALUES \
                 ('lattice_migrator'::text), ('lattice_runtime'::text), \
                 ('lattice_guardian'::text), ('lattice_readonly'::text), \
                 ('lattice_migrator_login'::text), ('lattice_runtime_login'::text), \
                 ('lattice_guardian_login'::text), ('lattice_readonly_login'::text) \
             ) \
             SELECT \
               (SELECT count(*) FROM resolved WHERE function_oid IS NULL), \
               (SELECT count(*) FROM resolved r \
                JOIN pg_proc p ON p.oid = r.function_oid \
                CROSS JOIN LATERAL aclexplode( \
                    COALESCE(p.proacl, acldefault('f', p.proowner)) \
                ) acl WHERE acl.grantee = 0), \
               (SELECT count(*) FROM resolved r CROSS JOIN fixed_roles f \
                WHERE has_function_privilege(f.role_name, r.function_oid, 'EXECUTE') \
                    <> COALESCE(r.allowed_role = f.role_name, false)), \
               (SELECT count(*) FROM resolved r \
                JOIN pg_proc p ON p.oid = r.function_oid \
                WHERE (SELECT count(*) \
                       FROM aclexplode(COALESCE(p.proacl, acldefault('f', p.proowner))) acl \
                       JOIN pg_roles grantee ON grantee.oid = acl.grantee \
                       WHERE grantee.rolname IN ('lattice_migrator', 'lattice_runtime', \
                           'lattice_guardian', 'lattice_readonly', \
                           'lattice_migrator_login', 'lattice_runtime_login', \
                           'lattice_guardian_login', 'lattice_readonly_login')) \
                       <> CASE WHEN r.allowed_role IS NULL THEN 0 ELSE 1 END \
                   OR (r.allowed_role IS NOT NULL AND NOT EXISTS ( \
                       SELECT 1 \
                       FROM aclexplode(COALESCE(p.proacl, acldefault('f', p.proowner))) acl \
                       JOIN pg_roles grantee ON grantee.oid = acl.grantee \
                       WHERE grantee.rolname = r.allowed_role \
                         AND acl.privilege_type = 'EXECUTE' \
                         AND NOT acl.is_grantable \
                         AND acl.grantor = p.proowner)))",
            &[],
        )
        .map_err(|error| {
            map_postgres_error(&error, PostgresStoreSetupErrorKind::PermissionDenied)
        })?;
    for index in 0..4 {
        if row_value::<i64>(
            &boundary,
            index,
            PostgresStoreSetupErrorKind::PermissionDenied,
        )? != 0
        {
            return Err(permission_error());
        }
    }
    Ok(())
}

fn verify_large_object_boundary<C: GenericClient>(
    client: &mut C,
) -> Result<(), PostgresStoreSetupError> {
    let forbidden = client
        .query_one(
            "WITH fixed_principals AS ( \
                 SELECT oid FROM pg_roles \
                 WHERE rolname IN ('lattice_migrator', 'lattice_runtime', \
                     'lattice_guardian', 'lattice_readonly', \
                     'lattice_migrator_login', 'lattice_runtime_login', \
                     'lattice_guardian_login', 'lattice_readonly_login') \
             ) \
             SELECT \
               (SELECT count(*) FROM pg_largeobject_metadata l \
                WHERE l.lomowner IN (SELECT oid FROM fixed_principals)), \
               (SELECT count(*) FROM pg_largeobject_metadata l \
                CROSS JOIN LATERAL aclexplode(l.lomacl) acl \
                WHERE acl.grantee = 0 \
                   OR acl.grantee IN (SELECT oid FROM fixed_principals))",
            &[],
        )
        .map_err(|error| {
            map_postgres_error(&error, PostgresStoreSetupErrorKind::PermissionDenied)
        })?;
    for index in 0..2 {
        if row_value::<i64>(
            &forbidden,
            index,
            PostgresStoreSetupErrorKind::PermissionDenied,
        )? != 0
        {
            return Err(permission_error());
        }
    }
    Ok(())
}

fn verify_login_principal_closure<C: GenericClient>(
    client: &mut C,
) -> Result<(), PostgresStoreSetupError> {
    let rows = client
        .query(
            "SELECT capability.rolname, login.rolname, m.admin_option, \
             m.inherit_option, m.set_option \
             FROM pg_auth_members m \
             JOIN pg_roles capability ON capability.oid = m.roleid \
             JOIN pg_roles login ON login.oid = m.member \
             WHERE capability.rolname LIKE 'lattice\\_%' ESCAPE '\\' \
                OR login.rolname LIKE 'lattice\\_%' ESCAPE '\\' \
             ORDER BY capability.rolname, login.rolname",
            &[],
        )
        .map_err(|error| {
            map_postgres_error(&error, PostgresStoreSetupErrorKind::PermissionDenied)
        })?;
    let expected = [
        ("lattice_guardian", "lattice_guardian_login"),
        ("lattice_migrator", "lattice_migrator_login"),
        ("lattice_readonly", "lattice_readonly_login"),
        ("lattice_runtime", "lattice_runtime_login"),
    ];
    if rows.len() != expected.len() {
        return Err(permission_error());
    }
    for (row, (expected_capability, expected_login)) in rows.iter().zip(expected) {
        let capability =
            row_value::<String>(row, 0, PostgresStoreSetupErrorKind::PermissionDenied)?;
        let login = row_value::<String>(row, 1, PostgresStoreSetupErrorKind::PermissionDenied)?;
        let admin = row_value::<bool>(row, 2, PostgresStoreSetupErrorKind::PermissionDenied)?;
        let inherit = row_value::<bool>(row, 3, PostgresStoreSetupErrorKind::PermissionDenied)?;
        let may_set = row_value::<bool>(row, 4, PostgresStoreSetupErrorKind::PermissionDenied)?;
        if capability != expected_capability
            || login != expected_login
            || admin
            || inherit
            || !may_set
        {
            return Err(permission_error());
        }
    }

    verify_login_database_acl(client)?;
    verify_login_object_closure(client)
}

fn verify_login_object_closure<C: GenericClient>(
    client: &mut C,
) -> Result<(), PostgresStoreSetupError> {
    let forbidden = client
        .query_one(
            "SELECT \
             (SELECT count(*) FROM pg_class c \
              WHERE c.relowner IN (SELECT oid FROM pg_roles WHERE rolname LIKE 'lattice\\_%\\_login' ESCAPE '\\')), \
             (SELECT count(*) FROM pg_namespace n \
              WHERE n.nspowner IN (SELECT oid FROM pg_roles WHERE rolname LIKE 'lattice\\_%\\_login' ESCAPE '\\')), \
             (SELECT count(*) FROM pg_proc p \
              WHERE p.proowner IN (SELECT oid FROM pg_roles WHERE rolname LIKE 'lattice\\_%\\_login' ESCAPE '\\')), \
             (SELECT count(*) FROM pg_type t \
              WHERE t.typowner IN (SELECT oid FROM pg_roles WHERE rolname LIKE 'lattice\\_%\\_login' ESCAPE '\\')), \
             (SELECT count(*) FROM pg_default_acl d \
              WHERE d.defaclrole IN (SELECT oid FROM pg_roles WHERE rolname LIKE 'lattice\\_%\\_login' ESCAPE '\\')), \
              (SELECT count(*) FROM pg_database d \
               WHERE d.datdba IN (SELECT oid FROM pg_roles WHERE rolname LIKE 'lattice\\_%\\_login' ESCAPE '\\')), \
              (SELECT count(*) FROM pg_shdepend d \
               WHERE d.refclassid = 'pg_authid'::regclass \
                 AND d.refobjid IN (SELECT oid FROM pg_roles WHERE rolname IN ( \
                     'lattice_migrator_login', 'lattice_runtime_login', \
                     'lattice_guardian_login', 'lattice_readonly_login')) \
                 AND d.deptype = 'o'), \
              (SELECT count(*) FROM ( \
                SELECT acl.grantee FROM pg_namespace n \
                CROSS JOIN LATERAL aclexplode(COALESCE(n.nspacl, acldefault('n', n.nspowner))) acl \
                UNION ALL SELECT acl.grantee FROM pg_class c \
                CROSS JOIN LATERAL aclexplode(COALESCE(c.relacl, acldefault('r', c.relowner))) acl \
                UNION ALL SELECT acl.grantee FROM pg_proc p \
                CROSS JOIN LATERAL aclexplode(COALESCE(p.proacl, acldefault('f', p.proowner))) acl \
                UNION ALL SELECT acl.grantee FROM pg_type t \
                CROSS JOIN LATERAL aclexplode(COALESCE(t.typacl, acldefault('T', t.typowner))) acl \
                UNION ALL SELECT acl.grantee FROM pg_attribute a \
                CROSS JOIN LATERAL aclexplode(a.attacl) acl \
                UNION ALL SELECT acl.grantee FROM pg_language l \
                CROSS JOIN LATERAL aclexplode(l.lanacl) acl \
                UNION ALL SELECT acl.grantee FROM pg_foreign_data_wrapper f \
                CROSS JOIN LATERAL aclexplode(f.fdwacl) acl \
                UNION ALL SELECT acl.grantee FROM pg_foreign_server s \
                CROSS JOIN LATERAL aclexplode(s.srvacl) acl \
                UNION ALL SELECT acl.grantee FROM pg_tablespace s \
                CROSS JOIN LATERAL aclexplode(s.spcacl) acl \
                UNION ALL SELECT acl.grantee FROM pg_largeobject_metadata l \
                CROSS JOIN LATERAL aclexplode(l.lomacl) acl \
                UNION ALL SELECT acl.grantee FROM pg_default_acl d \
                CROSS JOIN LATERAL aclexplode(d.defaclacl) acl \
              ) direct_acl \
              WHERE direct_acl.grantee IN (SELECT oid FROM pg_roles \
                  WHERE rolname LIKE 'lattice\\_%\\_login' ESCAPE '\\'))",
            &[],
        )
        .map_err(|error| {
            map_postgres_error(&error, PostgresStoreSetupErrorKind::PermissionDenied)
        })?;
    for index in 0..8 {
        if row_value::<i64>(
            &forbidden,
            index,
            PostgresStoreSetupErrorKind::PermissionDenied,
        )? != 0
        {
            return Err(permission_error());
        }
    }
    Ok(())
}

fn verify_login_database_acl<C: GenericClient>(
    client: &mut C,
) -> Result<(), PostgresStoreSetupError> {
    let direct_database_acl = client
        .query(
            "SELECT d.datname, d.datname = current_database(), grantee.rolname, \
             acl.privilege_type, acl.is_grantable, grantor.rolname \
             FROM pg_database d \
             CROSS JOIN LATERAL aclexplode(COALESCE(d.datacl, acldefault('d', d.datdba))) acl \
             JOIN pg_roles grantee ON grantee.oid = acl.grantee \
             JOIN pg_roles grantor ON grantor.oid = acl.grantor \
             WHERE grantee.rolname IN ('lattice_migrator_login', \
                 'lattice_runtime_login', 'lattice_guardian_login', \
                 'lattice_readonly_login') \
             ORDER BY d.datname, grantee.rolname, acl.privilege_type",
            &[],
        )
        .map_err(|error| {
            map_postgres_error(&error, PostgresStoreSetupErrorKind::PermissionDenied)
        })?;
    let expected_logins = [
        "lattice_guardian_login",
        "lattice_migrator_login",
        "lattice_readonly_login",
        "lattice_runtime_login",
    ];
    if direct_database_acl.len() != expected_logins.len() {
        return Err(permission_error());
    }
    for (row, expected_login) in direct_database_acl.iter().zip(expected_logins) {
        let is_target = row_value::<bool>(row, 1, PostgresStoreSetupErrorKind::PermissionDenied)?;
        let login = row_value::<String>(row, 2, PostgresStoreSetupErrorKind::PermissionDenied)?;
        let privilege = row_value::<String>(row, 3, PostgresStoreSetupErrorKind::PermissionDenied)?;
        let grantable = row_value::<bool>(row, 4, PostgresStoreSetupErrorKind::PermissionDenied)?;
        let grantor = row_value::<String>(row, 5, PostgresStoreSetupErrorKind::PermissionDenied)?;
        if !is_target
            || login != expected_login
            || privilege != "CONNECT"
            || grantable
            || grantor != DatabaseRole::Migrator.as_str()
        {
            return Err(permission_error());
        }
    }
    Ok(())
}

fn verify_nonwriter_capabilities<C: GenericClient>(
    client: &mut C,
    role: DatabaseRole,
    profile: CatalogProfile,
) -> Result<(), PostgresStoreSetupError> {
    let role_name = role.as_str();
    let boundary = client
        .query_one(
            "SELECT has_schema_privilege($1, 'control', 'USAGE'), \
             has_schema_privilege($1, 'control', 'CREATE'), \
             has_schema_privilege($1, 'memory', 'USAGE'), \
             has_schema_privilege($1, 'readmodel', 'USAGE'), \
             has_database_privilege($1, current_database(), 'CONNECT'), \
             has_database_privilege($1, current_database(), 'CREATE'), \
             has_database_privilege($1, current_database(), 'TEMPORARY'), \
             pg_has_role($1, 'lattice_migrator', 'MEMBER')",
            &[&role_name],
        )
        .map_err(|error| {
            map_postgres_error(&error, PostgresStoreSetupErrorKind::PermissionDenied)
        })?;
    let memory_usage = matches!(role, DatabaseRole::Runtime)
        && matches!(
            profile,
            CatalogProfile::V3CodebaseMemoryV2 | CatalogProfile::V3CodebaseMemoryV2WriterLeaseV1
        );
    let expected = [true, false, memory_usage, false, true, false, false, false];
    for (index, expected_value) in expected.into_iter().enumerate() {
        if row_value::<bool>(
            &boundary,
            index,
            PostgresStoreSetupErrorKind::PermissionDenied,
        )? != expected_value
        {
            return Err(permission_error());
        }
    }

    verify_nonwriter_global_capabilities(client, role)?;

    let protected_tables: Vec<&str> = match profile {
        CatalogProfile::V1 | CatalogProfile::V2 => PROTECTED_CONTROL_TABLES.into_iter().collect(),
        CatalogProfile::V3
        | CatalogProfile::V3CodebaseMemoryV2
        | CatalogProfile::V3CodebaseMemoryV2WriterLeaseV1 => {
            V3_PROTECTED_CONTROL_TABLES.into_iter().collect()
        }
        CatalogProfile::V4 => V4_PROTECTED_CONTROL_TABLES.into_iter().collect(),
        CatalogProfile::PreSchema => Vec::new(),
    };
    let mut bounded_tables: Vec<(&str, &str, bool)> = READABLE_CONTROL_TABLES
        .into_iter()
        .map(|table| ("control", table, true))
        .chain(
            protected_tables
                .into_iter()
                .map(|table| ("control", table, false)),
        )
        .collect();
    if matches!(
        profile,
        CatalogProfile::V3CodebaseMemoryV2 | CatalogProfile::V3CodebaseMemoryV2WriterLeaseV1
    ) {
        bounded_tables.extend(
            CODEBASE_MEMORY_V2_TABLES
                .into_iter()
                .map(|table| ("memory", table, false)),
        );
    }
    for (schema, table, may_read) in bounded_tables {
        let row = client
            .query_one(
                "SELECT has_table_privilege($1, c.oid, 'SELECT'), \
                 has_table_privilege($1, c.oid, 'INSERT'), \
                 has_table_privilege($1, c.oid, 'UPDATE'), \
                 has_table_privilege($1, c.oid, 'DELETE'), \
                 has_table_privilege($1, c.oid, 'TRUNCATE'), \
                 has_table_privilege($1, c.oid, 'REFERENCES'), \
                 has_table_privilege($1, c.oid, 'TRIGGER'), \
                 has_table_privilege($1, c.oid, 'MAINTAIN') \
                 FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace \
                 WHERE n.nspname = $2 AND c.relname = $3 AND c.relkind IN ('r', 'p')",
                &[&role_name, &schema, &table],
            )
            .map_err(|error| {
                map_postgres_error(&error, PostgresStoreSetupErrorKind::PermissionDenied)
            })?;
        if row_value::<bool>(&row, 0, PostgresStoreSetupErrorKind::PermissionDenied)? != may_read {
            return Err(permission_error());
        }
        for index in 1..=7 {
            if row_value::<bool>(&row, index, PostgresStoreSetupErrorKind::PermissionDenied)? {
                return Err(permission_error());
            }
        }
    }
    Ok(())
}

fn verify_nonwriter_global_capabilities<C: GenericClient>(
    client: &mut C,
    role: DatabaseRole,
) -> Result<(), PostgresStoreSetupError> {
    let login_name = role.login_role();
    let role_name = role.as_str();
    let global_boundary = client
        .query_one(
            "SELECT has_database_privilege($1, current_database(), 'CONNECT'), \
             has_database_privilege($1, current_database(), 'CREATE'), \
             has_database_privilege($1, current_database(), 'TEMPORARY'), \
             pg_has_role($1, $2, 'MEMBER'), pg_has_role($1, $2, 'USAGE'), \
             pg_has_role($1, $2, 'SET'), pg_has_role($2, $1, 'MEMBER'), \
             (SELECT count(*) FROM pg_namespace n \
              WHERE n.nspname !~ '^pg_' AND n.nspname <> 'information_schema' \
                AND has_schema_privilege($2, n.oid, 'CREATE')), \
             (SELECT count(*) FROM pg_namespace n \
              WHERE n.nspname !~ '^pg_' AND n.nspname <> 'information_schema' \
                AND has_schema_privilege($1, n.oid, 'CREATE')), \
             (SELECT count(*) FROM pg_namespace n \
              WHERE n.nspname !~ '^pg_' AND n.nspname <> 'information_schema' \
                AND has_schema_privilege('public', n.oid, 'CREATE'))",
            &[&login_name, &role_name],
        )
        .map_err(|error| {
            map_postgres_error(&error, PostgresStoreSetupErrorKind::PermissionDenied)
        })?;
    let login_boolean_expectations = [true, false, false, true, false, true, false];
    for (index, expected_value) in login_boolean_expectations.into_iter().enumerate() {
        if row_value::<bool>(
            &global_boundary,
            index,
            PostgresStoreSetupErrorKind::PermissionDenied,
        )? != expected_value
        {
            return Err(permission_error());
        }
    }
    for index in 7..=9 {
        if row_value::<i64>(
            &global_boundary,
            index,
            PostgresStoreSetupErrorKind::PermissionDenied,
        )? != 0
        {
            return Err(permission_error());
        }
    }
    Ok(())
}

fn verify_effective_default_privileges<C: GenericClient>(
    client: &mut C,
) -> Result<(), PostgresStoreSetupError> {
    let defaults = client
        .query_one(
            "SELECT \
             (SELECT count(*) \
             FROM (VALUES \
                  ('r'::\"char\", 'r'::\"char\"), \
                  ('S'::\"char\", 's'::\"char\"), \
                  ('f'::\"char\", 'f'::\"char\"), \
                  ('T'::\"char\", 'T'::\"char\") \
             ) AS expected(default_acl_type, acldefault_type) \
             JOIN pg_roles r ON r.rolname = 'lattice_migrator' \
             LEFT JOIN pg_default_acl d \
               ON d.defaclrole = r.oid \
              AND d.defaclnamespace = 0 \
              AND d.defaclobjtype = expected.default_acl_type \
             CROSS JOIN LATERAL aclexplode( \
                 COALESCE(d.defaclacl, acldefault(expected.acldefault_type, r.oid)) \
             ) a \
             WHERE a.grantee = 0), \
             (SELECT count(*) FROM pg_default_acl d \
             JOIN pg_namespace n ON n.oid = d.defaclnamespace \
             CROSS JOIN LATERAL aclexplode(d.defaclacl) a \
             WHERE d.defaclrole = 'lattice_migrator'::regrole \
             AND n.nspname IN ('control', 'memory', 'readmodel') \
             AND a.grantee = 0), \
             (SELECT count(*) FROM pg_default_acl d \
              CROSS JOIN LATERAL aclexplode(d.defaclacl) a \
              WHERE a.grantee = 0)",
            &[],
        )
        .map_err(|error| {
            map_postgres_error(&error, PostgresStoreSetupErrorKind::PermissionDenied)
        })?;
    let global_public_defaults =
        row_value::<i64>(&defaults, 0, PostgresStoreSetupErrorKind::PermissionDenied)?;
    let schema_public_defaults =
        row_value::<i64>(&defaults, 1, PostgresStoreSetupErrorKind::PermissionDenied)?;
    let all_owner_public_defaults =
        row_value::<i64>(&defaults, 2, PostgresStoreSetupErrorKind::PermissionDenied)?;
    if global_public_defaults != 0 || schema_public_defaults != 0 || all_owner_public_defaults != 0
    {
        return Err(permission_error());
    }
    Ok(())
}

fn string_set<C: GenericClient>(
    client: &mut C,
    query: &str,
) -> Result<BTreeSet<String>, PostgresStoreSetupError> {
    let rows = client
        .query(query, &[])
        .map_err(|error| map_postgres_error(&error, PostgresStoreSetupErrorKind::CorruptCatalog))?;
    let mut values = BTreeSet::new();
    for row in &rows {
        values.insert(row_value::<String>(
            row,
            0,
            PostgresStoreSetupErrorKind::CorruptCatalog,
        )?);
    }
    Ok(values)
}

fn catalog_signature<C: GenericClient>(
    client: &mut C,
    query: &str,
    error_kind: PostgresStoreSetupErrorKind,
) -> Result<String, PostgresStoreSetupError> {
    let rows = client
        .query(query, &[])
        .map_err(|error| map_postgres_error(&error, error_kind))?;
    let mut hasher = Sha256::new();
    hasher.update(CATALOG_SIGNATURE_DOMAIN);
    hasher.update(
        u64::try_from(rows.len())
            .map_err(|_| PostgresStoreSetupError::new(error_kind))?
            .to_be_bytes(),
    );
    for row in &rows {
        let value = row_value::<String>(row, 0, error_kind)?;
        hasher.update(
            u64::try_from(value.len())
                .map_err(|_| PostgresStoreSetupError::new(error_kind))?
                .to_be_bytes(),
        );
        hasher.update(value.as_bytes());
    }
    Ok(hex_digest(hasher.finalize().as_ref()))
}

fn hex_digest(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(64);
    for &byte in bytes {
        output.push(char::from(HEX[usize::from(byte >> 4)]));
        output.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    output
}

fn row_value<T: FromSqlOwned>(
    row: &Row,
    index: usize,
    error_kind: PostgresStoreSetupErrorKind,
) -> Result<T, PostgresStoreSetupError> {
    row.try_get(index)
        .map_err(|_| PostgresStoreSetupError::new(error_kind))
}

fn is_canonical_uuid(value: &str) -> bool {
    value.len() == 36
        && value.bytes().enumerate().all(|(index, byte)| {
            if matches!(index, 8 | 13 | 18 | 23) {
                byte == b'-'
            } else {
                byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
            }
        })
}

fn to_i16(value: u16) -> Result<i16, PostgresStoreSetupError> {
    i16::try_from(value)
        .map_err(|_| PostgresStoreSetupError::new(PostgresStoreSetupErrorKind::ManifestInvalid))
}

fn map_postgres_error(
    error: &postgres::Error,
    fallback: PostgresStoreSetupErrorKind,
) -> PostgresStoreSetupError {
    let kind = error.as_db_error().map_or(fallback, |database_error| {
        if database_error.code() == &SqlState::INSUFFICIENT_PRIVILEGE {
            PostgresStoreSetupErrorKind::PermissionDenied
        } else {
            fallback
        }
    });
    PostgresStoreSetupError::new(kind)
}

fn history_error() -> PostgresStoreSetupError {
    PostgresStoreSetupError::new(PostgresStoreSetupErrorKind::HistoryMismatch)
}

fn catalog_error() -> PostgresStoreSetupError {
    PostgresStoreSetupError::new(PostgresStoreSetupErrorKind::CorruptCatalog)
}

fn permission_error() -> PostgresStoreSetupError {
    PostgresStoreSetupError::new(PostgresStoreSetupErrorKind::PermissionDenied)
}

#[cfg(test)]
mod tests {
    use super::{
        CatalogProfile, REQUIRED_APPLICATION_NAME, apply_migrations, catalog_error,
        classify_current_catalog_profile, classify_extension_catalog_counts, is_loopback,
        read_database_identity, read_history_rows, verify_compatibility, verify_history_rows,
        verify_network_boundary, verify_roles_and_grants, verify_runtime_store_schema,
        verify_schema_objects, verify_server_version, verify_stopped_admission,
    };
    use crate::migrations::{
        DatabaseRole, MigrationTarget, PostgresStoreSetupError, PostgresStoreSetupErrorKind,
        migration_manifest, verify_v3_manifest_prefix,
    };
    use postgres::{Client, NoTls};

    const LIVE_PROFILE_GATE: &str = "LATTICE_STORE_PROFILE_LIVE";
    const LIVE_PROFILE_EXPECTED: &str = "LATTICE_STORE_PROFILE_EXPECTED";
    const LIVE_PROFILE_RUNTIME_URL: &str = "LATTICE_STORE_PROFILE_RUNTIME_URL";
    const LIVE_PROFILE_MIGRATOR_URL: &str = "LATTICE_STORE_PROFILE_MIGRATOR_URL";
    const LIVE_PROFILE_RUN_ID: &str = "LATTICE_TASK019_RUN_ID";

    struct LiveProfileFixture {
        target: MigrationTarget,
        runtime_url: String,
        migrator_url: String,
        expected_profile: CatalogProfile,
        expected_name: &'static str,
    }

    #[derive(Debug, Eq, PartialEq)]
    struct LiveDatabaseIdentity {
        database_name: String,
        database_uuid: String,
        server_address: String,
        server_port: i32,
    }

    struct LiveProfileMutation {
        apply_sql: &'static str,
        restore_sql: &'static str,
        expected_kind: PostgresStoreSetupErrorKind,
    }

    const WRITER_LEASE_PROFILE_MUTATIONS: [LiveProfileMutation; 9] = [
        LiveProfileMutation {
            apply_sql: "ALTER TABLE writer_lease.writer_lease_extension_ledger \
                        RENAME TO writer_lease_extension_ledger_profile_drift",
            restore_sql: "ALTER TABLE writer_lease.writer_lease_extension_ledger_profile_drift \
                          RENAME TO writer_lease_extension_ledger",
            expected_kind: PostgresStoreSetupErrorKind::CorruptCatalog,
        },
        LiveProfileMutation {
            apply_sql: "CREATE TABLE writer_lease.store_profile_live_extra (id bigint NOT NULL)",
            restore_sql: "DROP TABLE writer_lease.store_profile_live_extra",
            expected_kind: PostgresStoreSetupErrorKind::CorruptCatalog,
        },
        LiveProfileMutation {
            apply_sql: "ALTER TABLE writer_lease.writer_lease_extension_identity \
                        DROP CONSTRAINT writer_lease_extension_identity_singleton",
            restore_sql: "ALTER TABLE writer_lease.writer_lease_extension_identity \
                          ADD CONSTRAINT writer_lease_extension_identity_singleton \
                          CHECK (singleton)",
            expected_kind: PostgresStoreSetupErrorKind::CorruptCatalog,
        },
        LiveProfileMutation {
            apply_sql: "CREATE TYPE writer_lease.store_profile_live_extra_type AS ENUM ('DRIFT')",
            restore_sql: "DROP TYPE writer_lease.store_profile_live_extra_type",
            expected_kind: PostgresStoreSetupErrorKind::CorruptCatalog,
        },
        LiveProfileMutation {
            apply_sql: "ALTER FUNCTION writer_lease.writer_lease_load_current_v1(text) VOLATILE",
            restore_sql: "ALTER FUNCTION writer_lease.writer_lease_load_current_v1(text) STABLE",
            expected_kind: PostgresStoreSetupErrorKind::CorruptCatalog,
        },
        LiveProfileMutation {
            apply_sql: "GRANT USAGE ON SCHEMA writer_lease TO lattice_readonly",
            restore_sql: "REVOKE USAGE ON SCHEMA writer_lease FROM lattice_readonly",
            expected_kind: PostgresStoreSetupErrorKind::PermissionDenied,
        },
        LiveProfileMutation {
            apply_sql: "GRANT SELECT ON TABLE writer_lease.writer_lease_heads TO lattice_readonly",
            restore_sql: "REVOKE SELECT ON TABLE writer_lease.writer_lease_heads \
                          FROM lattice_readonly",
            expected_kind: PostgresStoreSetupErrorKind::PermissionDenied,
        },
        LiveProfileMutation {
            apply_sql: "GRANT EXECUTE ON FUNCTION \
                        writer_lease.writer_lease_load_current_v1(text) TO lattice_readonly",
            restore_sql: "REVOKE EXECUTE ON FUNCTION \
                          writer_lease.writer_lease_load_current_v1(text) FROM lattice_readonly",
            expected_kind: PostgresStoreSetupErrorKind::PermissionDenied,
        },
        LiveProfileMutation {
            apply_sql: "GRANT SELECT (project_id) ON TABLE \
                        writer_lease.writer_lease_heads TO lattice_readonly",
            restore_sql: "REVOKE SELECT (project_id) ON TABLE \
                          writer_lease.writer_lease_heads FROM lattice_readonly",
            expected_kind: PostgresStoreSetupErrorKind::PermissionDenied,
        },
    ];

    #[test]
    fn server_version_gate_rejects_every_non_seventeen_major() {
        assert!(verify_server_version(170_010).is_ok());
        for version in [0, 160_999, 180_000, u32::MAX] {
            assert_eq!(
                verify_server_version(version)
                    .expect_err("non-17 server must fail")
                    .kind(),
                PostgresStoreSetupErrorKind::ServerUnsupported
            );
        }
    }

    #[test]
    fn network_gate_requires_loopback_without_tls() {
        for (server, client, ssl) in [
            (None, Some("127.0.0.1"), false),
            (Some("127.0.0.1"), None, false),
            (Some("192.0.2.1"), Some("127.0.0.1"), false),
            (Some("127.0.0.1"), Some("192.0.2.1"), false),
            (Some("127.0.0.1"), Some("127.0.0.1"), true),
        ] {
            assert_eq!(
                verify_network_boundary(server, client, ssl)
                    .expect_err("unsafe network boundary must fail")
                    .kind(),
                PostgresStoreSetupErrorKind::NetworkBoundary
            );
        }
        assert!(verify_network_boundary(Some("127.0.0.1"), Some("::1"), false).is_ok());
    }

    #[test]
    fn extension_catalog_profile_accepts_only_closed_supported_combinations() {
        assert_eq!(
            classify_extension_catalog_counts(3, 0, 0, 0, 0, 0, 0, 0, 0, 0).expect("strict V3"),
            CatalogProfile::V3
        );
        assert_eq!(
            classify_extension_catalog_counts(3, 8, 8, 7, 7, 0, 0, 0, 0, 0)
                .expect("exact V3 Memory v2"),
            CatalogProfile::V3CodebaseMemoryV2
        );
        assert_eq!(
            classify_extension_catalog_counts(3, 8, 8, 7, 7, 1, 5, 5, 7, 7)
                .expect("exact V3 Memory v2 plus Writer Lease v1"),
            CatalogProfile::V3CodebaseMemoryV2WriterLeaseV1
        );
        assert_eq!(
            classify_extension_catalog_counts(4, 0, 0, 0, 0, 0, 0, 0, 0, 0).expect("strict V4"),
            CatalogProfile::V4
        );

        for counts in [
            (8, 8, 7, 7, 0, 0, 0, 0, 0),
            (8, 8, 7, 7, 1, 5, 5, 7, 7),
            (1, 1, 0, 0, 0, 0, 0, 0, 0),
            (7, 7, 6, 6, 0, 0, 0, 0, 0),
            (7, 7, 5, 5, 0, 0, 0, 0, 0),
            (7, 8, 6, 6, 0, 0, 0, 0, 0),
            (7, 7, 6, 7, 0, 0, 0, 0, 0),
            (8, 8, 6, 6, 0, 0, 0, 0, 0),
            (7, 7, 7, 7, 0, 0, 0, 0, 0),
            (0, 0, 0, 0, 1, 0, 0, 0, 0),
            (0, 0, 0, 0, 1, 5, 5, 7, 7),
            (8, 8, 7, 7, 0, 5, 5, 7, 7),
            (8, 8, 7, 7, 1, 4, 5, 7, 7),
            (8, 8, 7, 7, 1, 5, 5, 6, 7),
            (8, 8, 7, 7, 1, 5, 6, 7, 7),
            (8, 8, 7, 7, 1, 5, 5, 7, 8),
        ] {
            assert_eq!(
                classify_extension_catalog_counts(
                    4, counts.0, counts.1, counts.2, counts.3, counts.4, counts.5, counts.6,
                    counts.7, counts.8,
                )
                .expect_err("partial, unknown, extra, or overload must fail")
                .kind(),
                PostgresStoreSetupErrorKind::CorruptCatalog
            );
        }
    }

    #[test]
    fn live_store_profile_accepts_exact_profiles_and_rejects_writer_lease_drift_when_provisioned() {
        if std::env::var(LIVE_PROFILE_GATE).as_deref() != Ok("1") {
            eprintln!("SKIP: {LIVE_PROFILE_GATE} is not enabled");
            return;
        }

        let fixture = live_profile_fixture();
        let runtime_identity = assert_live_profile_accepted(&fixture);
        if matches!(
            fixture.expected_profile,
            CatalogProfile::V3CodebaseMemoryV2 | CatalogProfile::V3CodebaseMemoryV2WriterLeaseV1
        ) {
            assert_v3_extension_profile_rejected_as_upgrade_source(&fixture);
            assert_eq!(assert_live_profile_accepted(&fixture), runtime_identity);
        }
        if fixture.expected_profile == CatalogProfile::V3CodebaseMemoryV2WriterLeaseV1 {
            let migrator_url = required_live_environment(LIVE_PROFILE_MIGRATOR_URL);
            let mut migrator = connect_live_role(&migrator_url, DatabaseRole::Migrator);
            let migrator_identity = verify_live_connection_identity(
                &mut migrator,
                &fixture.target,
                DatabaseRole::Migrator,
            );
            assert!(
                migrator_identity == runtime_identity,
                "live profile runtime and migrator connections target different databases"
            );
            for mutation in &WRITER_LEASE_PROFILE_MUTATIONS {
                assert_live_profile_mutation_rejected_and_restored(
                    &fixture,
                    &mut migrator,
                    mutation,
                );
            }
        }

        eprintln!(
            "PASS: Store live profile {} accepted with exact fail-closed matrix",
            fixture.expected_name
        );
    }

    fn live_profile_fixture() -> LiveProfileFixture {
        let runtime_url = required_live_environment(LIVE_PROFILE_RUNTIME_URL);
        let run_id = required_live_environment(LIVE_PROFILE_RUN_ID);
        let expected = required_live_environment(LIVE_PROFILE_EXPECTED);
        let (expected_profile, expected_name) = match expected.as_str() {
            "V4" => (CatalogProfile::V4, "V4"),
            "V3_MEMORY_V2" => (CatalogProfile::V3CodebaseMemoryV2, "V3_MEMORY_V2"),
            "V3_MEMORY_V2_WRITER_LEASE_V1" => (
                CatalogProfile::V3CodebaseMemoryV2WriterLeaseV1,
                "V3_MEMORY_V2_WRITER_LEASE_V1",
            ),
            _ => panic!("unsupported Store live profile expectation"),
        };
        let database_name = live_database_name(&runtime_url);
        let target = MigrationTarget::new(database_name, run_id)
            .unwrap_or_else(|_| panic!("Store live profile target is not marker-owned disposable"));
        LiveProfileFixture {
            target,
            runtime_url,
            migrator_url: required_live_environment(LIVE_PROFILE_MIGRATOR_URL),
            expected_profile,
            expected_name,
        }
    }

    fn required_live_environment(name: &str) -> String {
        std::env::var(name)
            .ok()
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| panic!("required Store live profile environment is missing"))
    }

    fn live_database_name(url: &str) -> String {
        let config = url
            .parse::<postgres::Config>()
            .unwrap_or_else(|_| panic!("Store live profile connection configuration is invalid"));
        config
            .get_dbname()
            .filter(|name| !name.is_empty())
            .map_or_else(
                || panic!("Store live profile database name is missing"),
                str::to_owned,
            )
    }

    fn connect_live_role(url: &str, role: DatabaseRole) -> Client {
        let mut config = url
            .parse::<postgres::Config>()
            .unwrap_or_else(|_| panic!("Store live profile connection configuration is invalid"));
        config.application_name(REQUIRED_APPLICATION_NAME);
        let mut client = config
            .connect(NoTls)
            .unwrap_or_else(|_| panic!("Store live profile connection was rejected"));
        let set_role = match role {
            DatabaseRole::Migrator => "SET ROLE lattice_migrator",
            DatabaseRole::Runtime => "SET ROLE lattice_runtime",
            DatabaseRole::Guardian | DatabaseRole::ReadOnly => {
                panic!("unsupported Store live profile role")
            }
        };
        client
            .batch_execute(set_role)
            .unwrap_or_else(|_| panic!("Store live profile role transition was rejected"));
        client
    }

    fn verify_live_connection_identity(
        client: &mut Client,
        target: &MigrationTarget,
        role: DatabaseRole,
    ) -> LiveDatabaseIdentity {
        let row = client
            .query_one(
                "SELECT current_database()::text, current_user::text, session_user::text, \
                 inet_server_addr()::text, inet_client_addr()::text, inet_server_port(), \
                 current_setting('application_name'), \
                 COALESCE((SELECT ssl FROM pg_stat_ssl WHERE pid=pg_backend_pid()), false), \
                 (SELECT database_uuid::text FROM ONLY control.database_identity \
                   WHERE singleton=true)",
                &[],
            )
            .unwrap_or_else(|_| panic!("Store live profile connection identity was unavailable"));
        let database_name = live_row_value::<String>(&row, 0);
        let current_role = live_row_value::<String>(&row, 1);
        let session_role = live_row_value::<String>(&row, 2);
        let server_address = live_row_value::<String>(&row, 3);
        let client_address = live_row_value::<String>(&row, 4);
        let server_port = live_row_value::<i32>(&row, 5);
        let application_name = live_row_value::<String>(&row, 6);
        let ssl = live_row_value::<bool>(&row, 7);
        let database_uuid = live_row_value::<String>(&row, 8);
        assert!(
            database_name == target.database_name()
                && database_uuid == target.expected_database_uuid()
                && current_role == role.as_str()
                && session_role == role.login_role()
                && application_name == REQUIRED_APPLICATION_NAME
                && is_loopback(&server_address)
                && is_loopback(&client_address)
                && !ssl,
            "Store live profile connection identity failed closed"
        );
        LiveDatabaseIdentity {
            database_name,
            database_uuid,
            server_address,
            server_port,
        }
    }

    fn live_row_value<T: postgres::types::FromSqlOwned>(row: &postgres::Row, index: usize) -> T {
        row.try_get(index)
            .unwrap_or_else(|_| panic!("Store live profile row was malformed"))
    }

    fn assert_live_profile_accepted(fixture: &LiveProfileFixture) -> LiveDatabaseIdentity {
        if matches!(
            fixture.expected_profile,
            CatalogProfile::V3CodebaseMemoryV2 | CatalogProfile::V3CodebaseMemoryV2WriterLeaseV1
        ) {
            return assert_frozen_v3_extension_profile_accepted(fixture);
        }
        let mut runtime = connect_live_role(&fixture.runtime_url, DatabaseRole::Runtime);
        let identity =
            verify_live_connection_identity(&mut runtime, &fixture.target, DatabaseRole::Runtime);
        let profile = classify_current_catalog_profile(&mut runtime, 4)
            .unwrap_or_else(|_| panic!("Store live profile classification was rejected"));
        assert_eq!(profile, fixture.expected_profile);
        verify_runtime_store_schema(&mut runtime, &fixture.target)
            .unwrap_or_else(|_| panic!("exact Store live profile was rejected"));
        identity
    }

    fn verify_live_profile(fixture: &LiveProfileFixture) -> Result<(), PostgresStoreSetupError> {
        if matches!(
            fixture.expected_profile,
            CatalogProfile::V3CodebaseMemoryV2 | CatalogProfile::V3CodebaseMemoryV2WriterLeaseV1
        ) {
            return verify_frozen_v3_extension_profile(fixture).map(|_| ());
        }
        let mut runtime = connect_live_role(&fixture.runtime_url, DatabaseRole::Runtime);
        verify_runtime_store_schema(&mut runtime, &fixture.target).map(|_| ())
    }

    fn assert_frozen_v3_extension_profile_accepted(
        fixture: &LiveProfileFixture,
    ) -> LiveDatabaseIdentity {
        verify_frozen_v3_extension_profile(fixture)
            .unwrap_or_else(|_| panic!("frozen V3 Store extension profile was rejected"))
    }

    fn verify_frozen_v3_extension_profile(
        fixture: &LiveProfileFixture,
    ) -> Result<LiveDatabaseIdentity, PostgresStoreSetupError> {
        let v3_manifest = verify_v3_manifest_prefix()?;
        let mut migrator = connect_live_role(&fixture.migrator_url, DatabaseRole::Migrator);
        let identity =
            verify_live_connection_identity(&mut migrator, &fixture.target, DatabaseRole::Migrator);
        let profile = classify_current_catalog_profile(&mut migrator, 3)?;
        if profile != fixture.expected_profile {
            return Err(catalog_error());
        }
        verify_schema_objects(&mut migrator, profile)?;
        let rows = read_history_rows(&mut migrator)?;
        verify_history_rows(&rows, &migration_manifest()[..4])?;
        verify_compatibility(&mut migrator, &v3_manifest, profile)?;
        read_database_identity(&mut migrator, &fixture.target)?;
        verify_stopped_admission(&mut migrator)?;
        verify_roles_and_grants(&mut migrator, profile)?;
        Ok(identity)
    }

    fn assert_v3_extension_profile_rejected_as_upgrade_source(fixture: &LiveProfileFixture) {
        let mut migrator = connect_live_role(&fixture.migrator_url, DatabaseRole::Migrator);
        let rejected = apply_migrations(&mut migrator, &fixture.target)
            .expect_err("V3 extension profile must not be a V4 migration source");
        assert_eq!(
            rejected.kind(),
            PostgresStoreSetupErrorKind::CompatibilityMismatch
        );
    }

    fn assert_live_profile_mutation_rejected_and_restored(
        fixture: &LiveProfileFixture,
        migrator: &mut Client,
        mutation: &LiveProfileMutation,
    ) {
        migrator
            .batch_execute(mutation.apply_sql)
            .unwrap_or_else(|_| panic!("Store live profile fault injection was rejected"));
        let rejected = verify_live_profile(fixture);
        migrator
            .batch_execute(mutation.restore_sql)
            .unwrap_or_else(|_| panic!("Store live profile fault restoration was rejected"));
        let error =
            rejected.expect_err("drifted or over-privileged Store live profile must fail closed");
        assert_eq!(error.kind(), mutation.expected_kind);
        assert_live_profile_accepted(fixture);
    }
}
