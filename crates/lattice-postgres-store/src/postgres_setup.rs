//! Explicit `PostgreSQL` migration runner and read-only schema verifier.

use std::collections::BTreeSet;

use postgres::error::SqlState;
use postgres::types::FromSqlOwned;
use postgres::{Client, GenericClient, IsolationLevel, Row};
use sha2::{Digest, Sha256};

use crate::migrations::{
    CURRENT_V7_MANIFEST_SHA256, CURRENT_V8_MANIFEST_SHA256, DatabaseRole, ManifestEvidence,
    MigrationDescriptor, MigrationMetadata, MigrationStatus, MigrationTarget,
    MigrationTransactionMode, POSTGRES_SCHEMA_VERSION, PostgresStoreSetupError,
    PostgresStoreSetupErrorKind, STORE_V2_SCHEMA_VERSION, SUPPORTED_POSTGRES_MAJOR, Sha256Hex,
    migration_manifest, migration_metadata_sha256, verify_embedded_manifest,
    verify_v1_manifest_prefix, verify_v2_manifest_prefix, verify_v3_manifest_prefix,
    verify_v4_manifest_prefix, verify_v5_manifest_prefix, verify_v6_manifest_prefix,
    verify_v7_manifest_prefix, verify_v8_legacy_manifest_prefix,
};
use crate::schema_v6_profile::{
    FOREMAN_COORDINATION_EVENT_IDENTITY, FOREMAN_COORDINATION_STREAM_IDENTITY,
    ForemanSchemaV6Candidate, ForemanSchemaV6CatalogAcl, WriterLeaseV3Profile,
    verify_foreman_schema_v6_profile,
};

const MIGRATION_ADVISORY_LOCK: i64 = 0x4c41_5454_4943_4501;
const CODEBASE_MEMORY_ADVISORY_LOCK: i64 = 0x4c41_5443_4d45_4d31;
const WRITER_LEASE_ADVISORY_LOCK: i64 = 0x4c41_5457_4c45_4131;
const REQUIRED_APPLICATION_NAME: &str = "lattice-devos-task019";
const CODEBASE_MEMORY_EXTENSION_ID: &str = "lattice-codebase-memory";
const CODEBASE_MEMORY_V2_SCHEMA_VERSION: i16 = 2;
const CODEBASE_MEMORY_V2_PATH: &str = "db/extensions/codebase-memory/v2.sql";
const CODEBASE_MEMORY_V2_SQL_SHA256: &str =
    "9db54342b88f554ca76054c7a33ae72f04b412d2dfe21fae6eb4d8faf3e854e2";
const CODEBASE_MEMORY_V2_MANIFEST_SHA256: &str =
    "0aedbd7d9ef7ca07fc2910d0da34c163cc83e3dd56f9b28292ae1f4f0c3c4d7e";
const CODEBASE_MEMORY_V2_GLOBAL_SCHEMA_VERSION: i16 = 3;
const CODEBASE_MEMORY_V2_GLOBAL_MANIFEST_SHA256: &str =
    "09c431df18ad71a4f44239a5d2ddf6b1774b8ffec06c7f9223f0e41757f3d407";
const CODEBASE_MEMORY_V3_SCHEMA_VERSION: i16 = 3;
const CODEBASE_MEMORY_V3_PATH: &str = "db/extensions/codebase-memory/v3.sql";
const CODEBASE_MEMORY_V3_SQL_SHA256: &str =
    "7388f6bfe4c2d30a20306e4f9ebdff5862125bcab58f769ba286af542cb051c3";
const CODEBASE_MEMORY_V3_MANIFEST_SHA256: &str =
    "d4cc712d262ae1f7c96bd65526eab611c90e193363afd865af2126307b2903f0";
const WRITER_LEASE_V1_SQL_SHA256: &str =
    "63ffbf8f8b6c22bf35c3d393bd84e9462ca37e4ace94ceaedd6c27b729daa562";
const WRITER_LEASE_V2_SQL_SHA256: &str =
    "8243fd39a3565c641423fde3f15cf801a4a48a12c8d238ae8e1657acdcdc56e3";
const WRITER_LEASE_V3_SQL_SHA256: &str =
    "677c010a61e5945bcc6b96ca9f3d9e57830dc42f4cfbd46ea76d5e9d8b9262a0";
const WRITER_LEASE_V4_SQL_SHA256: &str =
    "51996b50c9a7d3696f8319613d35acae6257c5802b63dc4a809873721a22da09";
const WRITER_LEASE_V5_SQL_SHA256: &str =
    "c8193b47ef764d54a445a3f481331f642d0ce67b3a148c7c00fb3ca26d7ad12a";
const WRITER_LEASE_V5_STORE_V8_REBIND_SQL_SHA256: &str =
    "8916e5851d4def21808b4e7c78ba77d7a30a09f188222c604f64ad6d1463e7a4";
const WRITER_LEASE_V1_SQL: &str = include_str!("../../../db/extensions/writer-lease/v1.sql");
const WRITER_LEASE_V2_SQL: &str = include_str!("../../../db/extensions/writer-lease/v2.sql");
const WRITER_LEASE_V3_SQL: &str = include_str!("../../../db/extensions/writer-lease/v3.sql");
const WRITER_LEASE_V3_REBIND_SQL: &str =
    include_str!("../../../db/extensions/writer-lease/v3-rebind.sql");
const WRITER_LEASE_V4_SQL: &str = include_str!("../../../db/extensions/writer-lease/v4.sql");
const WRITER_LEASE_V4_REBIND_SQL: &str =
    include_str!("../../../db/extensions/writer-lease/v4-rebind.sql");
const WRITER_LEASE_V5_SQL: &str = include_str!("../../../db/extensions/writer-lease/v5.sql");
const WRITER_LEASE_V5_STORE_V8_REBIND_SQL: &str =
    include_str!("../../../db/extensions/writer-lease/v5-store-v8-rebind.sql");
const CODEBASE_MEMORY_V3_GLOBAL_SCHEMA_VERSION: i16 = 5;
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
    "3d1d5fcdf290c2b0fce7641951417e3371a06271b62abc710bc55265b6f87236";
const V3_PREFIX_EXPECTED_CONSTRAINT_SIGNATURE: &str =
    "f9d587125d792646b77ca68e6224c9866bd32c87a0e98c4d2f85b75dd0c22be8";
const V3_EXPECTED_INDEX_SIGNATURE: &str =
    "40ca5ea0781b1be03efe9bead50ae9f78434314123d6f700d278874678d06a9b";
const V3_CODEBASE_MEMORY_V2_EXPECTED_RELATION_SIGNATURE: &str =
    "162e7cdb50850fb31348e32ab4516a259fff2543d42fbfe2dd39e4f48679461d";
const V3_CODEBASE_MEMORY_V2_EXPECTED_COLUMN_SIGNATURE: &str =
    "6c71b33bb6ce0adda52c7267a2e15d0f76e80a7da8db847c87155c21db6b574b";
const V3_CODEBASE_MEMORY_V2_EXPECTED_CONSTRAINT_SIGNATURE: &str =
    "0b8c718a12925f0b88f24bf82b904fcff29d55203d42bb3412c5b05cca02e630";
const V3_CODEBASE_MEMORY_V2_PREFIX_EXPECTED_CONSTRAINT_SIGNATURE: &str =
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
const V5_EXPECTED_COLUMN_SIGNATURE: &str =
    "17391d4847127893e024301adbb13a4baf43dc941170ac2115fabc4d17f1bdd4";
const V5_EXPECTED_CONSTRAINT_SIGNATURE: &str =
    "21e34b2acd0573a92f217a059f72c22c3d2a40798bed7a66c140024ec796d409";
const V5_CODEBASE_MEMORY_V2_EXPECTED_RELATION_SIGNATURE: &str =
    "15daeb91d57a30edd2621d19556d5424e4df19d5a6609ab0c11e7dcedac3027b";
const V5_CODEBASE_MEMORY_V2_EXPECTED_COLUMN_SIGNATURE: &str =
    "39abf31761956d539e9c7d5061a718e5dee115efaa5c04cc5f16e50aa67f0856";
const V5_CODEBASE_MEMORY_V2_EXPECTED_CONSTRAINT_SIGNATURE: &str =
    "7c6e1c8c11c0d077fa85251174f217f6c3089d21439fb994784331376463ff5f";
const V5_CODEBASE_MEMORY_V2_EXPECTED_INDEX_SIGNATURE: &str =
    "0739c63458a2960cf80ca61383d727bbe93510bb677eeee9229b68019d12e7db";
const V5_CODEBASE_MEMORY_V3_EXPECTED_RELATION_SIGNATURE: &str =
    "15daeb91d57a30edd2621d19556d5424e4df19d5a6609ab0c11e7dcedac3027b";
const V5_CODEBASE_MEMORY_V3_EXPECTED_COLUMN_SIGNATURE: &str =
    "d8039c589202c1990be9bc9f5c650cda8a55e56420ec29511626afc99cbce5ef";
const V5_CODEBASE_MEMORY_V3_EXPECTED_CONSTRAINT_SIGNATURE: &str =
    "9915118318c192046dea5ead9b0d620181f128518c5009d0ffa3ef4b9c9d905c";
const V5_CODEBASE_MEMORY_V3_EXPECTED_INDEX_SIGNATURE: &str =
    "187ce7ae2cc9468e3d2eb22749a15525107f897ca28a5f1f7601623b7e90666b";
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
      AND NOT (n.nspname = 'control' AND c.relname = 'task_ledger_autonomy_receipts')
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
      AND NOT (n.nspname = 'control' AND c.relname = 'task_ledger_autonomy_receipts')
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
      AND NOT (n.nspname = 'control' AND c.relname = 'task_ledger_autonomy_receipts')
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
      AND NOT (n.nspname = 'control' AND table_class.relname = 'task_ledger_autonomy_receipts')
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
      AND NOT (n.nspname = 'control' AND c.relname = 'task_ledger_autonomy_receipts')
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
      AND NOT (n.nspname = 'control' AND t.typname IN (
          'task_ledger_autonomy_receipts', '_task_ledger_autonomy_receipts'
      ))
    ORDER BY n.nspname, t.typname
";

const TYPE_CATALOG_SIGNATURE_SQL: &str = r"
    SELECT jsonb_build_array(
        n.nspname, t.typname, t.typtype::text, t.typisdefined,
        owner.rolname, COALESCE(c.relname, '<NULL>'),
        COALESCE(element_ns.nspname || '.' || element.typname, '<NULL>'),
        COALESCE(array_ns.nspname || '.' || array_type.typname, '<NULL>')
    )::text
    FROM pg_type t
    JOIN pg_namespace n ON n.oid = t.typnamespace
    JOIN pg_roles owner ON owner.oid = t.typowner
    LEFT JOIN pg_class c ON c.oid = t.typrelid
    LEFT JOIN pg_type element ON element.oid = t.typelem
    LEFT JOIN pg_namespace element_ns ON element_ns.oid = element.typnamespace
    LEFT JOIN pg_type array_type ON array_type.oid = t.typarray
    LEFT JOIN pg_namespace array_ns ON array_ns.oid = array_type.typnamespace
    WHERE n.nspname IN ('control', 'memory', 'readmodel')
      AND NOT (n.nspname = 'control' AND t.typname IN (
          'task_ledger_autonomy_receipts', '_task_ledger_autonomy_receipts'
      ))
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
      AND NOT (n.nspname = 'control' AND p.proname IN (
          'task_ledger_record_autonomy_receipt_v1',
          'task_ledger_read_autonomy_receipts_v1'
      ))
    ORDER BY n.nspname, p.proname, pg_get_function_identity_arguments(p.oid)
";
const V2_EXPECTED_FUNCTION_SIGNATURE: &str =
    "1e0a53bff3e47d1accf1da1e0856b8edf77738fb5a20f9163b9d2d5747481064";
const V3_EXPECTED_FUNCTION_SIGNATURE: &str =
    "e38d34eb346dac00b3d2db13fe3b720f825d0d9d29f3c36e0e8b07b0892044ba";
const V3_PREFIX_EXPECTED_FUNCTION_SIGNATURE: &str =
    "f2c8585e1da944b38a50c65c6b9f448963f4c3d96c909331be87fec0c30d2279";
const V3_CODEBASE_MEMORY_V2_EXPECTED_FUNCTION_SIGNATURE: &str =
    "4dd4e12591231499501d37527ba2bb4f4a68bd25ddbab5cedc15264f2b39086a";
const V3_CODEBASE_MEMORY_V2_PREFIX_EXPECTED_FUNCTION_SIGNATURE: &str =
    "52e146d4e8190bf92ada1754f233423055a435cf281975f05fc83b262ff20db6";
const V4_EXPECTED_FUNCTION_SIGNATURE: &str =
    "557102df8882970df2c71a96b08998ee6d4c6a12d8cf312118ad80d8e1ad1c75";
const V5_EXPECTED_FUNCTION_SIGNATURE: &str =
    "b7d56229649886df72432ef296ccd81e0d93646b355fb387b02854a84dedeb4c";
const V5_CODEBASE_MEMORY_V2_EXPECTED_FUNCTION_SIGNATURE: &str =
    "6c627958a23e5602957fed0053b8aa840889cbc5aa48fabc45ca8314ddfcc132";
const V5_CODEBASE_MEMORY_V3_EXPECTED_FUNCTION_SIGNATURE: &str =
    "c26371ae6ad133bfc6fd619dadf03f7ecb9a4b862d6e9ad8d099beeaffa1328c";
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
      AND NOT (n.nspname = 'control' AND p.proname IN (
          'task_ledger_record_autonomy_receipt_v1',
          'task_ledger_read_autonomy_receipts_v1'
      ))
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
const V5_EXPECTED_FUNCTION_ACL_SIGNATURE: &str =
    "782a8869601b04ce61d722080db9e8742d26377afeff20718ca46e1948c19e43";
const V5_CODEBASE_MEMORY_V2_EXPECTED_FUNCTION_ACL_SIGNATURE: &str =
    "8a9ab2085db6ebc5b71c0fd3b92da3a499f999817e7e78322f0fe8ffe08f0982";
const V5_CODEBASE_MEMORY_V3_EXPECTED_FUNCTION_ACL_SIGNATURE: &str =
    "a13dc55ff84708829daa0e78494e12ae3aab66fae2ae6d0a89f5a617ad997b20";
const V3_EXPECTED_TABLE_ACL_SIGNATURE: &str =
    "27a0879d1b709abd341653b445d3a64d59819bde2e20e868ac09d2624aab1993";
const V3_CODEBASE_MEMORY_V2_EXPECTED_TABLE_ACL_SIGNATURE: &str =
    "273197d8086b87d4e3308afcc19e34d4b558c0723a23f6965fb07c8ad46f5770";
const V4_EXPECTED_TABLE_ACL_SIGNATURE: &str =
    "641f261e2cc1c93786eda9ac80fbcdb497e719708ad569bee65e9d451b43d2b0";
const V5_CODEBASE_MEMORY_V2_EXPECTED_TABLE_ACL_SIGNATURE: &str =
    "bb60c8b803c2eb2e6941f4523ff0eaaad804991bf674ee5a3d14331a262ab831";
const V5_CODEBASE_MEMORY_V3_EXPECTED_TABLE_ACL_SIGNATURE: &str =
    "bb60c8b803c2eb2e6941f4523ff0eaaad804991bf674ee5a3d14331a262ab831";

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

const ROLE_DATABASE_BOUNDARY_SQL: &str = "SELECT r.rolname, d.datistemplate, d.datallowconn, d.datconnlimit, \
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
     WHERE d.datname = current_database()";

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
const CODEBASE_MEMORY_V3_FUNCTIONS: [&str; 7] = [
    "codebase_memory_persist_analysis_v3",
    "codebase_memory_persist_retrieval_v3",
    "codebase_memory_load_receipt_v3",
    "codebase_memory_persist_reflection_v3",
    "codebase_memory_load_reflection_v3",
    "openclaw_gateway_reconcile_and_claim_v3",
    "openclaw_gateway_finalize_terminal_v3",
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
const WRITER_LEASE_V2_FUNCTIONS: [&str; 2] = [
    "writer_lease_bind_runtime_v2",
    "writer_lease_load_for_update_v2",
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
const WRITER_LEASE_V2_BRIDGE_CATALOG_PROFILES: [(&str, &str, PostgresStoreSetupErrorKind); 10] = [
    (
        WRITER_LEASE_V1_RELATION_PROFILE_SQL,
        "382b81889838d60c02ce5c31f77454e93f23372d90b3137a47663c5de74f9670",
        PostgresStoreSetupErrorKind::CorruptCatalog,
    ),
    (
        WRITER_LEASE_V1_COLUMN_PROFILE_SQL,
        "560e93c2a765db0024c0e74d25a51b90cfc72b204601139de8fdb688d48c0610",
        PostgresStoreSetupErrorKind::CorruptCatalog,
    ),
    (
        WRITER_LEASE_V1_CONSTRAINT_PROFILE_SQL,
        "3463b3ac82c1a7c53e5a80c41995f882ffe5f3f07fc5a82a97d50582d4d26915",
        PostgresStoreSetupErrorKind::CorruptCatalog,
    ),
    (
        WRITER_LEASE_V1_INDEX_PROFILE_SQL,
        "66b315513cbf50c3c7dbc143eb7061c6dbb823d7eac853c50f83434caf1a1022",
        PostgresStoreSetupErrorKind::CorruptCatalog,
    ),
    (
        WRITER_LEASE_V1_FUNCTION_PROFILE_SQL,
        "caa34168b5f9da4c8d2d02fce6e98882d73456c7c1f5c1af2b71f404efc647d1",
        PostgresStoreSetupErrorKind::CorruptCatalog,
    ),
    (
        WRITER_LEASE_V1_SCHEMA_ACL_PROFILE_SQL,
        "f8a84b870fcb8b091dbc7f9cf6835fb4311064eec5c83b31159a9a936a11e738",
        PostgresStoreSetupErrorKind::PermissionDenied,
    ),
    (
        WRITER_LEASE_V1_TABLE_ACL_PROFILE_SQL,
        "b99ef0c0ea5b550ae5e805d29b0020e31c1800a016b0de82cda566d7b25e9569",
        PostgresStoreSetupErrorKind::PermissionDenied,
    ),
    (
        WRITER_LEASE_V1_FUNCTION_ACL_PROFILE_SQL,
        "73951f1b33a4d6b3c4742fb49f91cf0601f04fd472b21c4db8bb36815fed0e89",
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
const WRITER_LEASE_V2_CURRENT_CATALOG_PROFILES: [(&str, &str, PostgresStoreSetupErrorKind); 10] = [
    (
        WRITER_LEASE_V1_RELATION_PROFILE_SQL,
        "382b81889838d60c02ce5c31f77454e93f23372d90b3137a47663c5de74f9670",
        PostgresStoreSetupErrorKind::CorruptCatalog,
    ),
    (
        WRITER_LEASE_V1_COLUMN_PROFILE_SQL,
        "560e93c2a765db0024c0e74d25a51b90cfc72b204601139de8fdb688d48c0610",
        PostgresStoreSetupErrorKind::CorruptCatalog,
    ),
    (
        WRITER_LEASE_V1_CONSTRAINT_PROFILE_SQL,
        "3463b3ac82c1a7c53e5a80c41995f882ffe5f3f07fc5a82a97d50582d4d26915",
        PostgresStoreSetupErrorKind::CorruptCatalog,
    ),
    (
        WRITER_LEASE_V1_INDEX_PROFILE_SQL,
        "66b315513cbf50c3c7dbc143eb7061c6dbb823d7eac853c50f83434caf1a1022",
        PostgresStoreSetupErrorKind::CorruptCatalog,
    ),
    (
        WRITER_LEASE_V1_FUNCTION_PROFILE_SQL,
        "caa34168b5f9da4c8d2d02fce6e98882d73456c7c1f5c1af2b71f404efc647d1",
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
        "bd5b05d60340a1b9f9fbf1de2b4bed8586b7eede4fd8d7c4825841c221e89b7a",
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
const WRITER_LEASE_V3_CURRENT_CATALOG_SIGNATURES: [&str; 10] = [
    "0fdd123e2939cee6ad128564668ef57c8130d0780ee9f6a3a7f725d1c4ce840f",
    "560e93c2a765db0024c0e74d25a51b90cfc72b204601139de8fdb688d48c0610",
    "9be3f0f60b5113317678328d490ff88d866c252c3544e5bed1a7a60c0d543cc1",
    "66b315513cbf50c3c7dbc143eb7061c6dbb823d7eac853c50f83434caf1a1022",
    "4b9a0caf84307961d7780f87ca0aa9e0382c9920def32415d3688fb28e7701ef",
    "a2e1be8a403a96b679c18ddfa75e476fa1d6ceeccc1ccf62ff6424b2c259ef7b",
    "b99ef0c0ea5b550ae5e805d29b0020e31c1800a016b0de82cda566d7b25e9569",
    "72640b1eec7e3bbb4e56532f795712930dd84f79f6d2ea846bd83395185fdbf3",
    "a7ccfc938fbf121a9b807070f69bd5b851be6aa89a8261043ef07336ea7b8dbd",
    "1d6642e77600a93da5b00dda0ee64c15474b4ca2741c51ca760597e7f90ac003",
];
const WRITER_LEASE_V4_BRIDGE_CATALOG_SIGNATURES: [&str; 10] = [
    "41652b9772ad01aeb834f84eb0fc21ecef8a424afa755a8ec1b95e86eedb8861",
    "560e93c2a765db0024c0e74d25a51b90cfc72b204601139de8fdb688d48c0610",
    "8aec9d54882a49e41b93bc1ead82f80c34e52c679f3e9efaca45342325af622e",
    "66b315513cbf50c3c7dbc143eb7061c6dbb823d7eac853c50f83434caf1a1022",
    "e253b95704f78e288fc0a0799327a7034932cc8e721b390f9d629cccebc3a8d0",
    "f8a84b870fcb8b091dbc7f9cf6835fb4311064eec5c83b31159a9a936a11e738",
    "b99ef0c0ea5b550ae5e805d29b0020e31c1800a016b0de82cda566d7b25e9569",
    "25de9857318874012f716bb9a9db146aa40597cd8fe95ff550ba21f55f2dcc00",
    "a7ccfc938fbf121a9b807070f69bd5b851be6aa89a8261043ef07336ea7b8dbd",
    "1d6642e77600a93da5b00dda0ee64c15474b4ca2741c51ca760597e7f90ac003",
];
const WRITER_LEASE_V4_CURRENT_CATALOG_SIGNATURES: [&str; 10] = [
    "41652b9772ad01aeb834f84eb0fc21ecef8a424afa755a8ec1b95e86eedb8861",
    "560e93c2a765db0024c0e74d25a51b90cfc72b204601139de8fdb688d48c0610",
    "8aec9d54882a49e41b93bc1ead82f80c34e52c679f3e9efaca45342325af622e",
    "66b315513cbf50c3c7dbc143eb7061c6dbb823d7eac853c50f83434caf1a1022",
    "e253b95704f78e288fc0a0799327a7034932cc8e721b390f9d629cccebc3a8d0",
    "a2e1be8a403a96b679c18ddfa75e476fa1d6ceeccc1ccf62ff6424b2c259ef7b",
    "b99ef0c0ea5b550ae5e805d29b0020e31c1800a016b0de82cda566d7b25e9569",
    "42db5b1428da3e9e6aa96f770dd1996893fdcdf4f88b275d1ddb28ae8df12309",
    "a7ccfc938fbf121a9b807070f69bd5b851be6aa89a8261043ef07336ea7b8dbd",
    "1d6642e77600a93da5b00dda0ee64c15474b4ca2741c51ca760597e7f90ac003",
];
const WRITER_LEASE_V5_CURRENT_CATALOG_SIGNATURES: [&str; 10] = [
    "7f60105269127d4351cdd00cdff7d8cb23230c2420ce4cd24ff3746ac7763e37",
    "560e93c2a765db0024c0e74d25a51b90cfc72b204601139de8fdb688d48c0610",
    "348feffe66ee0e4cb8f26183f0515c11a792a18ec0e761dfd410b6e09c16a5dd",
    "66b315513cbf50c3c7dbc143eb7061c6dbb823d7eac853c50f83434caf1a1022",
    "2ec0db61c83d6090bbad13beaad2d07e9362d75a4853346e80e988aee3cd1252",
    "a2e1be8a403a96b679c18ddfa75e476fa1d6ceeccc1ccf62ff6424b2c259ef7b",
    "b99ef0c0ea5b550ae5e805d29b0020e31c1800a016b0de82cda566d7b25e9569",
    "d33ac72256c97191ffbb7f7e74d9908b92d9b9fc865801c3f3301c50fdd4e34b",
    "a7ccfc938fbf121a9b807070f69bd5b851be6aa89a8261043ef07336ea7b8dbd",
    "1d6642e77600a93da5b00dda0ee64c15474b4ca2741c51ca760597e7f90ac003",
];
const WRITER_LEASE_V5_STORE_V8_CURRENT_CATALOG_SIGNATURES: [&str; 10] = [
    "7f60105269127d4351cdd00cdff7d8cb23230c2420ce4cd24ff3746ac7763e37",
    "560e93c2a765db0024c0e74d25a51b90cfc72b204601139de8fdb688d48c0610",
    "348feffe66ee0e4cb8f26183f0515c11a792a18ec0e761dfd410b6e09c16a5dd",
    "66b315513cbf50c3c7dbc143eb7061c6dbb823d7eac853c50f83434caf1a1022",
    "a7e45c42d51cffe5b5d30f1c6097c515a3b26271570a886004cf3a40f27734a7",
    "a2e1be8a403a96b679c18ddfa75e476fa1d6ceeccc1ccf62ff6424b2c259ef7b",
    "b99ef0c0ea5b550ae5e805d29b0020e31c1800a016b0de82cda566d7b25e9569",
    "d33ac72256c97191ffbb7f7e74d9908b92d9b9fc865801c3f3301c50fdd4e34b",
    "a7ccfc938fbf121a9b807070f69bd5b851be6aa89a8261043ef07336ea7b8dbd",
    "1d6642e77600a93da5b00dda0ee64c15474b4ca2741c51ca760597e7f90ac003",
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
const TASK_LEDGER_RECORD_AUTONOMY_RECEIPT_V1_IDENTITY: &str = "control.task_ledger_record_autonomy_receipt_v1(bytea,text,bytea,text,text,text,text,boolean,boolean,boolean,text,text,text,text,text,text,bytea,bytea,bytea,bytea,bytea,text,bytea,bytea)";
const TASK_LEDGER_READ_AUTONOMY_RECEIPTS_V1_IDENTITY: &str =
    "control.task_ledger_read_autonomy_receipts_v1(bytea)";
const AUTONOMY_PROFILE_SIGNATURE_SQL: &str = r"
    SELECT signature::text
      FROM (
        SELECT 0 AS ordinal,
               jsonb_build_array(
                   'relation', n.nspname, c.relname, owner.rolname,
                   c.relkind::text, c.relpersistence::text, c.relrowsecurity,
                   c.relforcerowsecurity, c.relhassubclass, c.relispartition,
                   COALESCE(c.relacl::text, '<NULL>'),
                   COALESCE((
                       SELECT jsonb_agg(jsonb_build_array(
                           a.attnum, a.attname,
                           pg_catalog.format_type(a.atttypid, a.atttypmod),
                           a.attlen, a.attndims, a.attbyval, a.attalign::text,
                           a.attstorage::text, a.attcompression::text,
                           a.attnotnull, a.atthasdef, a.attidentity::text,
                           a.attgenerated::text,
                           COALESCE(coll_ns.nspname || '.' || coll.collname, '<NULL>'),
                           COALESCE(pg_catalog.pg_get_expr(def.adbin, def.adrelid), '<NULL>')
                       ) ORDER BY a.attnum)
                         FROM pg_catalog.pg_attribute a
                         LEFT JOIN pg_catalog.pg_attrdef def
                           ON def.adrelid = a.attrelid AND def.adnum = a.attnum
                         LEFT JOIN pg_catalog.pg_collation coll ON coll.oid = a.attcollation
                         LEFT JOIN pg_catalog.pg_namespace coll_ns
                           ON coll_ns.oid = coll.collnamespace
                        WHERE a.attrelid = c.oid AND a.attnum > 0
                   ), '[]'::jsonb),
                   COALESCE((
                       SELECT jsonb_agg(jsonb_build_array(
                           con.conname, con.contype::text, con.condeferrable,
                           con.condeferred, con.convalidated,
                           pg_catalog.pg_get_constraintdef(con.oid, true)
                       ) ORDER BY con.conname)
                         FROM pg_catalog.pg_constraint con
                        WHERE con.conrelid = c.oid
                   ), '[]'::jsonb),
                   COALESCE((
                       SELECT jsonb_agg(pg_catalog.pg_get_indexdef(i.indexrelid)
                                        ORDER BY ix.relname)
                         FROM pg_catalog.pg_index i
                         JOIN pg_catalog.pg_class ix ON ix.oid = i.indexrelid
                        WHERE i.indrelid = c.oid
                   ), '[]'::jsonb)
               ) AS signature
          FROM pg_catalog.pg_class c
          JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace
          JOIN pg_catalog.pg_roles owner ON owner.oid = c.relowner
         WHERE n.nspname = 'control'
           AND c.relname = 'task_ledger_autonomy_receipts'
        UNION ALL
        SELECT 1 AS ordinal,
               jsonb_build_array(
                   'function', n.nspname, p.proname,
                   pg_catalog.pg_get_function_identity_arguments(p.oid),
                   pg_catalog.pg_get_function_result(p.oid), owner.rolname,
                   language.lanname, p.prokind::text, p.prosecdef, p.proleakproof,
                   p.provolatile::text, p.proparallel::text, p.proisstrict,
                   p.proretset, p.pronargs, p.pronargdefaults,
                   p.prorettype::regtype::text, p.proargtypes::text,
                   COALESCE(p.proallargtypes::text, '<NULL>'),
                   COALESCE(p.proargmodes::text, '<NULL>'),
                   COALESCE(p.proargnames::text, '<NULL>'),
                   COALESCE(array_to_string(p.proconfig, ','), '<NULL>'),
                   COALESCE(p.proacl::text, '<NULL>'), COALESCE(p.probin, '<NULL>'),
                   p.prosrc, pg_catalog.pg_get_functiondef(p.oid)
               ) AS signature
          FROM pg_catalog.pg_proc p
          JOIN pg_catalog.pg_namespace n ON n.oid = p.pronamespace
          JOIN pg_catalog.pg_roles owner ON owner.oid = p.proowner
          JOIN pg_catalog.pg_language language ON language.oid = p.prolang
         WHERE n.nspname = 'control'
           AND p.proname IN ('task_ledger_record_autonomy_receipt_v1',
                             'task_ledger_read_autonomy_receipts_v1')
      ) exact_profile
     ORDER BY ordinal, signature::text
";
const AUTONOMY_PROFILE_SIGNATURE: &str =
    "e4995f4127a8ad1fb7123c78ef4de3990d5d6493cbe999a860a6f7407577035d";
const V7_AMBIGUITY_RELATION_SIGNATURE_SQL: &str = r"
    SELECT jsonb_build_array(
        n.nspname, c.relname, c.relkind::text, owner.rolname,
        c.relpersistence::text, c.relrowsecurity, c.relforcerowsecurity,
        c.relhassubclass, c.relispartition, c.relreplident::text,
        COALESCE(array_to_string(c.reloptions, ','), '<NULL>')
    )::text
    FROM pg_catalog.pg_class c
    JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace
    JOIN pg_catalog.pg_roles owner ON owner.oid=c.relowner
    WHERE n.nspname='control'
      AND c.relname='task_ingress_historical_ambiguities'
    ORDER BY n.nspname,c.relname
";
const V7_AMBIGUITY_COLUMN_SIGNATURE_SQL: &str = r"
    SELECT jsonb_build_array(
        n.nspname,c.relname,a.attnum,a.attname,
        pg_catalog.format_type(a.atttypid,a.atttypmod),a.attnotnull,a.attisdropped,
        COALESCE(pg_catalog.pg_get_expr(def.adbin,def.adrelid,false),'<NULL>'),
        a.attidentity::text,a.attgenerated::text,
        CASE WHEN coll.oid IS NULL THEN '<NULL>'
             ELSE coll_ns.nspname || '.' || coll.collname END,
        a.attstorage::text,a.attcompression::text,a.attstattarget,
        COALESCE(a.attacl::text,'<NULL>')
    )::text
    FROM pg_catalog.pg_class c
    JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace
    JOIN pg_catalog.pg_attribute a ON a.attrelid=c.oid
    LEFT JOIN pg_catalog.pg_attrdef def
      ON def.adrelid=c.oid AND def.adnum=a.attnum
    LEFT JOIN pg_catalog.pg_collation coll ON coll.oid=a.attcollation
    LEFT JOIN pg_catalog.pg_namespace coll_ns ON coll_ns.oid=coll.collnamespace
    WHERE n.nspname='control'
      AND c.relname='task_ingress_historical_ambiguities'
      AND c.relkind='r' AND a.attnum>0
    ORDER BY a.attnum
";
const V7_AMBIGUITY_CONSTRAINT_SIGNATURE_SQL: &str = r"
    SELECT jsonb_build_array(
        n.nspname,c.relname,con.conname,con.contype::text,
        con.convalidated,con.condeferrable,con.condeferred,
        con.connoinherit,con.conislocal,con.coninhcount,
        con.conkey,ref_ns.nspname,ref_class.relname,con.confkey,
        con.confupdtype::text,con.confdeltype::text,con.confmatchtype::text,
        pg_catalog.pg_get_constraintdef(con.oid,false)
    )::text
    FROM pg_catalog.pg_constraint con
    JOIN pg_catalog.pg_namespace n ON n.oid=con.connamespace
    JOIN pg_catalog.pg_class c ON c.oid=con.conrelid
    LEFT JOIN pg_catalog.pg_class ref_class ON ref_class.oid=con.confrelid
    LEFT JOIN pg_catalog.pg_namespace ref_ns ON ref_ns.oid=ref_class.relnamespace
    WHERE n.nspname='control'
      AND c.relname='task_ingress_historical_ambiguities'
    ORDER BY con.conname
";
const V7_AMBIGUITY_INDEX_SIGNATURE_SQL: &str = r"
    SELECT jsonb_build_array(
        n.nspname,table_class.relname,index_class.relname,
        idx.indisunique,idx.indisprimary,idx.indisvalid,idx.indisready,
        idx.indislive,idx.indisclustered,idx.indisreplident,
        idx.indnullsnotdistinct,
        pg_catalog.pg_get_indexdef(idx.indexrelid,0,true)
    )::text
    FROM pg_catalog.pg_index idx
    JOIN pg_catalog.pg_class table_class ON table_class.oid=idx.indrelid
    JOIN pg_catalog.pg_class index_class ON index_class.oid=idx.indexrelid
    JOIN pg_catalog.pg_namespace n ON n.oid=table_class.relnamespace
    WHERE n.nspname='control'
      AND table_class.relname='task_ingress_historical_ambiguities'
    ORDER BY index_class.relname
";
const V7_AMBIGUITY_TABLE_ACL_SIGNATURE_SQL: &str = r"
    SELECT jsonb_build_array(
        n.nspname,c.relname,owner.rolname,COALESCE(grantee.rolname,'PUBLIC'),
        grantor.rolname,acl.privilege_type,acl.is_grantable
    )::text
    FROM pg_catalog.pg_class c
    JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace
    JOIN pg_catalog.pg_roles owner ON owner.oid=c.relowner
    CROSS JOIN LATERAL pg_catalog.aclexplode(
        COALESCE(c.relacl,pg_catalog.acldefault('r',c.relowner))
    ) acl
    LEFT JOIN pg_catalog.pg_roles grantee ON grantee.oid=acl.grantee
    JOIN pg_catalog.pg_roles grantor ON grantor.oid=acl.grantor
    WHERE n.nspname='control'
      AND c.relname='task_ingress_historical_ambiguities'
    ORDER BY owner.rolname,COALESCE(grantee.rolname,'PUBLIC'),
             grantor.rolname,acl.privilege_type,acl.is_grantable
";
const V7_INGRESS_FUNCTION_SIGNATURE_SQL: &str = r"
    SELECT jsonb_build_array(
        n.nspname,p.proname,pg_catalog.pg_get_function_identity_arguments(p.oid),
        pg_catalog.pg_get_function_result(p.oid),owner.rolname,language.lanname,
        p.prokind::text,p.prosecdef,p.proleakproof,p.provolatile::text,
        p.proparallel::text,p.proisstrict,p.proretset,p.pronargs,
        p.pronargdefaults,p.prorettype::regtype::text,p.proargtypes::text,
        COALESCE(p.proallargtypes::text,'<NULL>'),
        COALESCE(p.proargmodes::text,'<NULL>'),
        COALESCE(p.proargnames::text,'<NULL>'),
        COALESCE(pg_catalog.array_to_string(p.proconfig,','),'<NULL>'),
        COALESCE(p.probin,'<NULL>'),p.prosrc,pg_catalog.pg_get_functiondef(p.oid)
    )::text
    FROM pg_catalog.pg_proc p
    JOIN pg_catalog.pg_namespace n ON n.oid=p.pronamespace
    JOIN pg_catalog.pg_roles owner ON owner.oid=p.proowner
    JOIN pg_catalog.pg_language language ON language.oid=p.prolang
    WHERE n.nspname='control'
      AND p.proname IN (
          'task_ingress_prepare_v1','task_ingress_record_v1',
          'task_ingress_read_by_request_v1','task_ingress_historical_closure_v1'
      )
    ORDER BY p.proname,pg_catalog.pg_get_function_identity_arguments(p.oid)
";
const V7_INGRESS_FUNCTION_ACL_SIGNATURE_SQL: &str = r"
    SELECT jsonb_build_array(
        n.nspname,p.proname,pg_catalog.pg_get_function_identity_arguments(p.oid),
        COALESCE(grantee.rolname,'PUBLIC'),grantor.rolname,
        acl.privilege_type,acl.is_grantable
    )::text
    FROM pg_catalog.pg_proc p
    JOIN pg_catalog.pg_namespace n ON n.oid=p.pronamespace
    CROSS JOIN LATERAL pg_catalog.aclexplode(
        COALESCE(p.proacl,pg_catalog.acldefault('f',p.proowner))
    ) acl
    LEFT JOIN pg_catalog.pg_roles grantee ON grantee.oid=acl.grantee
    JOIN pg_catalog.pg_roles grantor ON grantor.oid=acl.grantor
    WHERE n.nspname='control'
      AND p.proname IN (
          'task_ingress_prepare_v1','task_ingress_record_v1',
          'task_ingress_read_by_request_v1','task_ingress_historical_closure_v1'
      )
    ORDER BY p.proname,pg_catalog.pg_get_function_identity_arguments(p.oid),
             COALESCE(grantee.rolname,'PUBLIC'),grantor.rolname,
             acl.privilege_type,acl.is_grantable
";
const V7_AMBIGUITY_RELATION_SIGNATURE: &str =
    "53cc68fd7aeeb3af0148baff87bc4eacc1b939926763ab9e5469d77a49ff2305";
const V7_AMBIGUITY_COLUMN_SIGNATURE: &str =
    "1dcb03efc12cc644df4c245fe455e0f1dd2120cc4cf78490f64b1a9b36592579";
const V7_AMBIGUITY_CONSTRAINT_SIGNATURE: &str =
    "3d025c0526ea179600c72b2a4f47720495fd51b0100fcc85a702c79b622175f1";
const V7_AMBIGUITY_INDEX_SIGNATURE: &str =
    "76fe47e1b086dcceba180e905f1e21ea1dcf0749cef49b0e2cfb8a71e397dfb2";
const V7_AMBIGUITY_TABLE_ACL_SIGNATURE: &str =
    "5e64fab71687e50674f06ab8d4904d5d3bb22b314bc1a7fa052c7d7b319a85eb";
const V7_INGRESS_FUNCTION_SIGNATURE: &str =
    "f4d55ccc4782025666fcdee2cf68380d2ebf1b8533224af02ff4f778a14a3124";
const V7_INGRESS_FUNCTION_ACL_SIGNATURE: &str =
    "7d8c5087a4561b8149ea2fc0d76b7310c6e94f2a8d162e37a4533e4c97e32ca4";
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
const STORE_PREPARE_V5_IDENTITY: &str = "control.store_prepare_v5(smallint,text,smallint,text,text,text,text,bytea,bytea,text,text,bigint,text,bigint,bytea,bytea,text,bigint,bytea,bytea,bytea,bytea,bytea,bytea,bytea,bytea,bytea,bytea)";
const STORE_FINALIZE_V5_IDENTITY: &str = "control.store_finalize_v5(smallint,text,smallint,text,text,text,text,bytea,bytea,text,text,bigint,text,bigint,bytea,bytea,text,bigint,bytea,bytea,bytea,bytea,bytea,bytea,bytea,bytea,bytea,bytea,uuid,bytea,smallint,text,bigint,bytea,bytea,bigint,bytea,bytea,text,bytea,bytea)";
const STORE_CURRENT_HEAD_V5_IDENTITY: &str =
    "control.store_current_head_v5(smallint,text,text,text,text,bytea)";
const TASK_LEDGER_PREPARE_V3_IDENTITY: &str =
    "control.task_ledger_prepare_v3(smallint,text,bytea,text)";
const TASK_LEDGER_READ_HEAD_V3_IDENTITY: &str =
    "control.task_ledger_read_head_v3(smallint,text,bytea,text,text)";
const TASK_LEDGER_READ_EVENTS_V3_IDENTITY: &str =
    "control.task_ledger_read_events_v3(smallint,text,bytea)";
const TASK_LEDGER_READ_COMMANDS_V3_IDENTITY: &str =
    "control.task_ledger_read_commands_v3(smallint,text,bytea)";
const TASK_LEDGER_FINALIZE_V3_IDENTITY: &str = "control.task_ledger_finalize_v3(smallint,text,bytea,text,text,text,text,bytea,text,text,bytea,text,bytea,bytea,text,text,text,text,text,text,text,text,text,bytea,bytea,text,bytea,text,bytea,text,bytea,bytea,text,text,text,text,text,text,text,bytea,jsonb,boolean,text,text,text,text,text,text,text,bytea,text,bytea,bytea,text,bytea,text,bytea,bytea,text,text,bytea,bytea,bytea,text,boolean,text,bytea,text,bytea,boolean,bytea,bytea)";
const PROJECT_REGISTRY_PREPARE_V2_IDENTITY: &str = "control.project_registry_prepare_v2(smallint,text,text,bytea,text,text,bigint,text,bigint,bytea,bytea,bytea)";
const PROJECT_REGISTRY_READ_STATE_V2_IDENTITY: &str =
    "control.project_registry_read_state_v2(smallint,text)";
const PROJECT_REGISTRY_READ_OBSERVATIONS_V2_IDENTITY: &str =
    "control.project_registry_read_observations_v2(smallint,text)";
const PROJECT_REGISTRY_READ_PROJECTS_V2_IDENTITY: &str =
    "control.project_registry_read_projects_v2(smallint,text)";
const PROJECT_REGISTRY_READ_COMMANDS_V2_IDENTITY: &str =
    "control.project_registry_read_commands_v2(smallint,text)";
const PROJECT_REGISTRY_READ_RESERVATIONS_V2_IDENTITY: &str =
    "control.project_registry_read_reservations_v2(smallint,text)";
const PROJECT_REGISTRY_STAGE_COMMAND_V2_IDENTITY: &str = "control.project_registry_stage_command_v2(smallint,text,smallint,text,bigint,text,text,text,text,bytea,boolean,text,text,text,text,text,numeric,text,text,text,bytea,bytea,bytea,text,bytea,bytea,text,text,text,text,text,text,text,bytea,bytea,bytea,boolean,boolean,boolean,boolean,boolean,bytea,text,bigint,bigint,bigint,bigint,bigint,bigint,bytea,text,bigint,bigint,bigint,bigint,bigint,bigint,bytea,bytea,text,text,bigint,text,bigint,bytea,bytea,bytea,bytea,boolean,text,bytea,bytea,bytea,text,bytea)";
const PROJECT_REGISTRY_STAGE_PROJECT_V2_IDENTITY: &str = "control.project_registry_stage_project_v2(smallint,text,text,text,bytea,bytea,boolean,boolean,boolean,boolean,boolean,smallint,text,text,text,text,numeric,text,text,bytea,bytea,bytea)";
const PROJECT_REGISTRY_FINALIZE_V2_IDENTITY: &str = "control.project_registry_finalize_v2(smallint,text,text,bigint,text,bigint,bigint,bigint,bigint,bigint,bigint,bytea,text,bigint,bigint,bigint,bigint,bigint,bigint,bytea,bytea,bytea,bytea,boolean,boolean,bigint,bigint)";
const V5_RUNTIME_FUNCTION_IDENTITIES: [&str; 19] = [
    STORE_PREPARE_V5_IDENTITY,
    STORE_FINALIZE_V5_IDENTITY,
    STORE_CURRENT_HEAD_V5_IDENTITY,
    TASK_LEDGER_PREPARE_V3_IDENTITY,
    TASK_LEDGER_READ_HEAD_V3_IDENTITY,
    TASK_LEDGER_READ_EVENTS_V3_IDENTITY,
    TASK_LEDGER_READ_COMMANDS_V3_IDENTITY,
    TASK_LEDGER_FINALIZE_V3_IDENTITY,
    PROJECT_REGISTRY_PREPARE_V2_IDENTITY,
    PROJECT_REGISTRY_READ_STATE_V2_IDENTITY,
    PROJECT_REGISTRY_READ_OBSERVATIONS_V2_IDENTITY,
    PROJECT_REGISTRY_READ_PROJECTS_V2_IDENTITY,
    PROJECT_REGISTRY_READ_COMMANDS_V2_IDENTITY,
    PROJECT_REGISTRY_READ_RESERVATIONS_V2_IDENTITY,
    PROJECT_REGISTRY_STAGE_COMMAND_V2_IDENTITY,
    PROJECT_REGISTRY_STAGE_PROJECT_V2_IDENTITY,
    PROJECT_REGISTRY_FINALIZE_V2_IDENTITY,
    TASK_LEDGER_RECORD_AUTONOMY_RECEIPT_V1_IDENTITY,
    TASK_LEDGER_READ_AUTONOMY_RECEIPTS_V1_IDENTITY,
];
const V5_SUCCESSOR_FUNCTION_IDENTITIES: [&str; 17] = [
    STORE_PREPARE_V5_IDENTITY,
    STORE_FINALIZE_V5_IDENTITY,
    STORE_CURRENT_HEAD_V5_IDENTITY,
    TASK_LEDGER_PREPARE_V3_IDENTITY,
    TASK_LEDGER_READ_HEAD_V3_IDENTITY,
    TASK_LEDGER_READ_EVENTS_V3_IDENTITY,
    TASK_LEDGER_READ_COMMANDS_V3_IDENTITY,
    TASK_LEDGER_FINALIZE_V3_IDENTITY,
    PROJECT_REGISTRY_PREPARE_V2_IDENTITY,
    PROJECT_REGISTRY_READ_STATE_V2_IDENTITY,
    PROJECT_REGISTRY_READ_OBSERVATIONS_V2_IDENTITY,
    PROJECT_REGISTRY_READ_PROJECTS_V2_IDENTITY,
    PROJECT_REGISTRY_READ_COMMANDS_V2_IDENTITY,
    PROJECT_REGISTRY_READ_RESERVATIONS_V2_IDENTITY,
    PROJECT_REGISTRY_STAGE_COMMAND_V2_IDENTITY,
    PROJECT_REGISTRY_STAGE_PROJECT_V2_IDENTITY,
    PROJECT_REGISTRY_FINALIZE_V2_IDENTITY,
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
const CODEBASE_MEMORY_V2_FUNCTION_IDENTITIES: [&str; 7] = [
    CODEBASE_MEMORY_LOAD_REFLECTION_V2_IDENTITY,
    CODEBASE_MEMORY_LOAD_RECEIPT_V1_IDENTITY,
    CODEBASE_MEMORY_PERSIST_ANALYSIS_V1_IDENTITY,
    CODEBASE_MEMORY_PERSIST_REFLECTION_V2_IDENTITY,
    CODEBASE_MEMORY_PERSIST_RETRIEVAL_V1_IDENTITY,
    OPENCLAW_GATEWAY_FINALIZE_TERMINAL_V1_IDENTITY,
    OPENCLAW_GATEWAY_RECONCILE_AND_CLAIM_V1_IDENTITY,
];
const CODEBASE_MEMORY_V3_FUNCTION_IDENTITIES: [&str; 7] = [
    "memory.codebase_memory_load_reflection_v3(bytea,bytea,bytea,bytea,smallint,text,text,text,text,bytea,text,text,bytea,bytea,smallint)",
    "memory.codebase_memory_load_receipt_v3(bytea,bytea,bytea,bytea,smallint,text,text,text,text,bytea,text,text,bytea,bytea,smallint)",
    "memory.codebase_memory_persist_analysis_v3(bytea,bytea,bytea,bytea,smallint,text,text,text,text,bytea,text,text,bytea,bytea,smallint,text,bytea,bytea,bytea,bytea,bytea,bytea,bytea,bytea,bytea,integer[],bytea[],text[],text[],text[],text[],text[],text[],bytea[],integer[],integer[],text[],bytea[])",
    "memory.codebase_memory_persist_reflection_v3(bytea,bytea,bytea,bytea,smallint,text,text,text,text,bytea,text,text,bytea,bytea,smallint,bytea,text,text,bytea,bytea,bytea,bytea,text,text[],bytea[],text[])",
    "memory.codebase_memory_persist_retrieval_v3(bytea,bytea,bytea,bytea,bytea,bytea,bytea,smallint,text,bytea[],bytea[],bigint[],bytea,bytea,bytea)",
    "memory.openclaw_gateway_finalize_terminal_v3(bytea,bytea,bytea,bytea,text,text,bigint,text,bytea,bytea,bytea,bytea)",
    "memory.openclaw_gateway_reconcile_and_claim_v3(bytea,bytea,bytea,bytea,text,text,bigint,text,bytea)",
];
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

/// Read-only migration prefix used by the product bootstrap coordinator.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MigrationBootstrapProfile {
    Fresh,
    LegacyPrefix,
    V5,
    V6,
    V7,
    V8LegacyPrefix,
    V8,
}

/// Classifies only an exact embedded migration prefix without changing it.
///
/// # Errors
///
/// Rejects partial schemas, changed history, target drift, unavailable
/// evidence, or an invalid migrator boundary.
pub fn inspect_migration_profile(
    client: &mut Client,
    target: &MigrationTarget,
) -> Result<MigrationBootstrapProfile, PostgresStoreSetupError> {
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
        DatabaseRole::Migrator,
        SetupOperation::Verification,
    )?;
    let profile = match classify_installed_manifest_state(&mut transaction)? {
        InstalledManifestState::Fresh => MigrationBootstrapProfile::Fresh,
        InstalledManifestState::ExactV1Prefix
        | InstalledManifestState::ExactV2Prefix
        | InstalledManifestState::ExactV3Prefix
        | InstalledManifestState::ExactV4Prefix => MigrationBootstrapProfile::LegacyPrefix,
        InstalledManifestState::ExactV5Prefix => MigrationBootstrapProfile::V5,
        InstalledManifestState::ExactV6Prefix => MigrationBootstrapProfile::V6,
        InstalledManifestState::ExactV7Prefix => MigrationBootstrapProfile::V7,
        InstalledManifestState::ExactV8LegacyPrefix => MigrationBootstrapProfile::V8LegacyPrefix,
        InstalledManifestState::ExactV8Full => MigrationBootstrapProfile::V8,
    };
    preflight_connection(
        &mut transaction,
        target,
        DatabaseRole::Migrator,
        SetupOperation::Verification,
    )?;
    transaction.commit().map_err(|error| {
        map_postgres_error(&error, PostgresStoreSetupErrorKind::TransactionFailed)
    })?;
    Ok(profile)
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RetainedHistoryClassification {
    ExactSupported,
    StrictFutureSuffix,
    Corrupt,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct RetainedMigrationHistoryRow {
    ordinal: i16,
    migration_id: String,
    migration_path: String,
    byte_length: i64,
    checksum_sha256: String,
    migration_status: String,
    transaction_mode: String,
    schema_version: i16,
    min_reader: i16,
    max_reader: i16,
    min_writer: i16,
    max_writer: i16,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct RetainedSchemaCompatibility {
    manifest_sha256: String,
    versions: [i16; 5],
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
    ExactV4Prefix,
    ExactV5Prefix,
    ExactV6Prefix,
    ExactV7Prefix,
    ExactV8LegacyPrefix,
    ExactV8Full,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CatalogProfile {
    PreSchema,
    V1,
    V2,
    V3,
    V3CodebaseMemoryV2,
    V3CodebaseMemoryV2WriterLeaseV1,
    V3CodebaseMemoryV2WriterLeaseV2Bridge,
    V4,
    V5,
    V5CodebaseMemoryV2UpgradePending,
    V5CodebaseMemoryV3Current,
    V5CodebaseMemoryV2WriterLeaseV2BridgePending,
    V5CodebaseMemoryV3WriterLeaseV2BridgePending,
    V5CodebaseMemoryV3WriterLeaseV2Current,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum WriterLeaseV2RuntimeProfile {
    Bridge,
    Current,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum WriterLeaseV5RuntimeProfile {
    StoreV7Base,
    StoreV8Successor,
}

#[allow(clippy::too_many_arguments)]
fn classify_extension_catalog_counts(
    schema_version: u16,
    expected_memory_relations: i64,
    all_memory_relations: i64,
    expected_memory_v2_functions: i64,
    expected_memory_v3_functions: i64,
    all_memory_functions: i64,
    writer_lease_namespaces: i64,
    expected_writer_lease_relations: i64,
    all_writer_lease_relations: i64,
    expected_writer_lease_functions: i64,
    all_writer_lease_functions: i64,
    runtime_writer_lease_functions: i64,
    runtime_writer_lease_schema_usage: bool,
) -> Result<CatalogProfile, PostgresStoreSetupError> {
    let no_memory = expected_memory_relations == 0
        && all_memory_relations == 0
        && expected_memory_v2_functions == 0
        && expected_memory_v3_functions == 0
        && all_memory_functions == 0;
    let exact_memory_v2 = expected_memory_relations
        == i64::try_from(CODEBASE_MEMORY_V2_TABLES.len()).expect("fixed count")
        && all_memory_relations == expected_memory_relations
        && expected_memory_v2_functions
            == i64::try_from(CODEBASE_MEMORY_V2_FUNCTIONS.len()).expect("fixed count")
        && expected_memory_v3_functions == 0
        && all_memory_functions == expected_memory_v2_functions;
    let exact_memory_v3 = expected_memory_relations
        == i64::try_from(CODEBASE_MEMORY_V2_TABLES.len()).expect("fixed count")
        && all_memory_relations == expected_memory_relations
        && expected_memory_v2_functions
            == i64::try_from(CODEBASE_MEMORY_V2_FUNCTIONS.len()).expect("fixed count")
        && expected_memory_v3_functions
            == i64::try_from(CODEBASE_MEMORY_V3_FUNCTIONS.len()).expect("fixed count")
        && all_memory_functions == expected_memory_v2_functions + expected_memory_v3_functions;
    let no_writer_lease = writer_lease_namespaces == 0
        && expected_writer_lease_relations == 0
        && all_writer_lease_relations == 0
        && expected_writer_lease_functions == 0
        && all_writer_lease_functions == 0
        && runtime_writer_lease_functions == 0
        && !runtime_writer_lease_schema_usage;
    let exact_writer_lease_v1 = writer_lease_namespaces == 1
        && expected_writer_lease_relations
            == i64::try_from(WRITER_LEASE_V1_TABLES.len()).expect("fixed count")
        && all_writer_lease_relations == expected_writer_lease_relations
        && expected_writer_lease_functions
            == i64::try_from(WRITER_LEASE_V1_FUNCTIONS.len()).expect("fixed count")
        && all_writer_lease_functions == expected_writer_lease_functions
        && runtime_writer_lease_functions
            == i64::try_from(WRITER_LEASE_V1_FUNCTIONS.len()).expect("fixed count")
        && runtime_writer_lease_schema_usage;
    let exact_writer_lease_v2_shape = writer_lease_namespaces == 1
        && expected_writer_lease_relations
            == i64::try_from(WRITER_LEASE_V1_TABLES.len()).expect("fixed count")
        && all_writer_lease_relations == expected_writer_lease_relations
        && expected_writer_lease_functions
            == i64::try_from(WRITER_LEASE_V1_FUNCTIONS.len() + WRITER_LEASE_V2_FUNCTIONS.len())
                .expect("fixed count")
        && all_writer_lease_functions == expected_writer_lease_functions;
    let exact_writer_lease_v2_bridge = exact_writer_lease_v2_shape
        && runtime_writer_lease_functions == 0
        && !runtime_writer_lease_schema_usage;
    let exact_writer_lease_v2_current = exact_writer_lease_v2_shape
        && runtime_writer_lease_functions
            == i64::try_from(WRITER_LEASE_V1_FUNCTIONS.len()).expect("fixed count")
        && runtime_writer_lease_schema_usage;

    if schema_version == 3 && no_memory && no_writer_lease {
        return Ok(CatalogProfile::V3);
    }
    if schema_version == 3 && exact_memory_v2 && no_writer_lease {
        return Ok(CatalogProfile::V3CodebaseMemoryV2);
    }
    if schema_version == 3 && exact_memory_v2 && exact_writer_lease_v1 {
        return Ok(CatalogProfile::V3CodebaseMemoryV2WriterLeaseV1);
    }
    if schema_version == 3 && exact_memory_v2 && exact_writer_lease_v2_bridge {
        return Ok(CatalogProfile::V3CodebaseMemoryV2WriterLeaseV2Bridge);
    }
    if schema_version == 4 && no_memory && no_writer_lease {
        return Ok(CatalogProfile::V4);
    }
    if schema_version == 5 && no_memory && no_writer_lease {
        return Ok(CatalogProfile::V5);
    }
    if schema_version == 5 && exact_memory_v2 && no_writer_lease {
        return Ok(CatalogProfile::V5CodebaseMemoryV2UpgradePending);
    }
    if schema_version == 5 && exact_memory_v2 && exact_writer_lease_v2_bridge {
        return Ok(CatalogProfile::V5CodebaseMemoryV2WriterLeaseV2BridgePending);
    }
    if schema_version == 5 && exact_memory_v3 && no_writer_lease {
        return Ok(CatalogProfile::V5CodebaseMemoryV3Current);
    }
    if schema_version == 5 && exact_memory_v3 && exact_writer_lease_v2_bridge {
        return Ok(CatalogProfile::V5CodebaseMemoryV3WriterLeaseV2BridgePending);
    }
    if schema_version == 5 && exact_memory_v3 && exact_writer_lease_v2_current {
        return Ok(CatalogProfile::V5CodebaseMemoryV3WriterLeaseV2Current);
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
             count(*) FILTER (WHERE p.proname IN ( \
                 'codebase_memory_persist_analysis_v3', \
                 'codebase_memory_persist_retrieval_v3', \
                 'codebase_memory_load_receipt_v3', \
                 'codebase_memory_persist_reflection_v3', \
                 'codebase_memory_load_reflection_v3', \
                 'openclaw_gateway_reconcile_and_claim_v3', \
                 'openclaw_gateway_finalize_terminal_v3'))::bigint, \
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
        row_value::<i64>(&row, 2, PostgresStoreSetupErrorKind::CorruptCatalog)?,
        writer_lease_counts.0,
        writer_lease_counts.1,
        writer_lease_counts.2,
        writer_lease_counts.3,
        writer_lease_counts.4,
        writer_lease_counts.5,
        writer_lease_counts.6,
    )
}

fn writer_lease_catalog_counts<C: GenericClient>(
    client: &mut C,
) -> Result<WriterLeaseCatalogCounts, PostgresStoreSetupError> {
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
                 'writer_lease_bind_runtime_v2', \
                 'writer_lease_commit_plan_v1', \
                 'writer_lease_load_commands_v1', \
                 'writer_lease_load_current_v1', \
                 'writer_lease_load_for_update_v1', \
                 'writer_lease_load_for_update_v2', \
                 'writer_lease_load_transitions_v1'))::bigint \
                FROM pg_proc p JOIN pg_namespace function_ns ON function_ns.oid = p.pronamespace \
               WHERE function_ns.nspname = 'writer_lease'), \
             (SELECT count(*)::bigint \
                FROM pg_proc p JOIN pg_namespace function_ns ON function_ns.oid = p.pronamespace \
               WHERE function_ns.nspname = 'writer_lease'), \
             (SELECT count(*) FILTER (WHERE \
                       pg_catalog.has_function_privilege('lattice_runtime', p.oid, 'EXECUTE'))::bigint \
                FROM pg_proc p JOIN pg_namespace function_ns ON function_ns.oid = p.pronamespace \
               WHERE function_ns.nspname = 'writer_lease'), \
             COALESCE((SELECT pg_catalog.has_schema_privilege( \
                              'lattice_runtime', writer_ns.oid, 'USAGE') \
                         FROM pg_namespace writer_ns \
                        WHERE writer_ns.nspname = 'writer_lease'), false) \
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
        row_value::<i64>(&row, 5, PostgresStoreSetupErrorKind::PermissionDenied)?,
        row_value::<bool>(&row, 6, PostgresStoreSetupErrorKind::PermissionDenied)?,
    ))
}

type WriterLeaseCatalogCounts = (i64, i64, i64, i64, i64, i64, bool);

/// Applies only the exact embedded executable manifest under the migration role.
///
/// # Errors
///
/// Fails closed with a bounded static error before mutation for an invalid
/// manifest, target, role, server, network, or setting. Pre-commit transaction
/// failures roll back; a commit error is reported as an unknown outcome. Once
/// commit succeeds, a verifier failure is reported separately as committed but
/// unverified and exact manifest retry is required for reconciliation.
#[allow(clippy::too_many_lines)]
pub fn apply_migrations(
    client: &mut Client,
    target: &MigrationTarget,
) -> Result<MigrationApplyOutcome, PostgresStoreSetupError> {
    let manifest = verify_embedded_manifest()?;
    let legacy_manifest = verify_v1_manifest_prefix()?;
    let store_v2_manifest = verify_v2_manifest_prefix()?;
    let v3_manifest = verify_v3_manifest_prefix()?;
    let v4_manifest = verify_v4_manifest_prefix()?;
    let v5_manifest = verify_v5_manifest_prefix()?;
    let v6_manifest = verify_v6_manifest_prefix()?;
    let v7_manifest = verify_v7_manifest_prefix()?;
    let v8_legacy_manifest = verify_v8_legacy_manifest_prefix()?;
    let mut transaction = client
        .build_transaction()
        .isolation_level(IsolationLevel::ReadCommitted)
        .start()
        .map_err(|error| {
            map_postgres_error(&error, PostgresStoreSetupErrorKind::TransactionFailed)
        })?;
    harden_transaction(&mut transaction)?;
    for advisory_lock in [
        &MIGRATION_ADVISORY_LOCK,
        &CODEBASE_MEMORY_ADVISORY_LOCK,
        &WRITER_LEASE_ADVISORY_LOCK,
    ] {
        transaction
            .query_one("SELECT pg_advisory_xact_lock($1)", &[advisory_lock])
            .map_err(|error| {
                map_postgres_error(&error, PostgresStoreSetupErrorKind::TransactionFailed)
            })?;
    }
    preflight_connection(
        &mut transaction,
        target,
        DatabaseRole::Migrator,
        SetupOperation::Migration,
    )?;

    let installed = classify_installed_manifest_state(&mut transaction)?;
    let outcome = match installed {
        InstalledManifestState::Fresh => {
            verify_role_and_database_boundary(&mut transaction, CatalogProfile::PreSchema, false)?;
            let executable_count = apply_entries_until(&mut transaction, 0, 6)?;
            seed_database_identity(&mut transaction, target)?;
            insert_current_compatibility(&mut transaction, &v5_manifest, 6)?;
            MigrationApplyOutcome::Applied { executable_count }
        }
        InstalledManifestState::ExactV1Prefix => {
            verify_v1_upgrade_source(&mut transaction, &legacy_manifest, target)?;
            let executable_count = apply_entries_until(&mut transaction, 2, 6)?;
            advance_compatibility_from_v1(&mut transaction, &legacy_manifest, &v5_manifest)?;
            MigrationApplyOutcome::Applied { executable_count }
        }
        InstalledManifestState::ExactV2Prefix => {
            verify_v2_upgrade_source(&mut transaction, &store_v2_manifest, target)?;
            let executable_count = apply_entries_until(&mut transaction, 3, 6)?;
            advance_compatibility_from_v2(&mut transaction, &store_v2_manifest, &v5_manifest)?;
            MigrationApplyOutcome::Applied { executable_count }
        }
        InstalledManifestState::ExactV3Prefix => {
            verify_v3_upgrade_source(&mut transaction, &v3_manifest, target)?;
            let executable_count = apply_entries_until(&mut transaction, 4, 6)?;
            advance_compatibility_from_v3(&mut transaction, &v3_manifest, &v5_manifest)?;
            MigrationApplyOutcome::Applied { executable_count }
        }
        InstalledManifestState::ExactV4Prefix => {
            verify_v4_upgrade_source(&mut transaction, &v4_manifest, target)?;
            let executable_count = apply_entries_until(&mut transaction, 5, 6)?;
            advance_compatibility_from_v4(&mut transaction, &v4_manifest, &v5_manifest)?;
            MigrationApplyOutcome::Applied { executable_count }
        }
        InstalledManifestState::ExactV5Prefix => {
            verify_v5_upgrade_source(&mut transaction, &v5_manifest, target)?;
            let executable_count = apply_entries_until(&mut transaction, 6, 7)?;
            advance_compatibility_from_v5(&mut transaction, &v5_manifest, &v6_manifest)?;
            transaction
                .batch_execute("CALL writer_lease.writer_lease_rebind_v3()")
                .map_err(|error| {
                    map_postgres_error(&error, PostgresStoreSetupErrorKind::TransactionFailed)
                })?;
            verify_runtime_foreman_schema_v6(
                &mut transaction,
                target,
                &v6_manifest,
                false,
                SchemaV6WriterProfile::V3Current,
            )?;
            MigrationApplyOutcome::Applied { executable_count }
        }
        InstalledManifestState::ExactV6Prefix => {
            verify_runtime_foreman_schema_v6(
                &mut transaction,
                target,
                &v6_manifest,
                false,
                SchemaV6WriterProfile::V4Bridge,
            )?;
            let executable_count = apply_entries_until(&mut transaction, 7, 8)?;
            advance_compatibility_from_v6(&mut transaction, &v6_manifest, &v7_manifest)?;
            transaction
                .batch_execute("CALL writer_lease.writer_lease_rebind_v4()")
                .map_err(|error| {
                    map_postgres_error(&error, PostgresStoreSetupErrorKind::TransactionFailed)
                })?;
            verify_runtime_submission_schema_v7(&mut transaction, target, &v7_manifest, false)?;
            MigrationApplyOutcome::Applied { executable_count }
        }
        InstalledManifestState::ExactV7Prefix => {
            verify_runtime_submission_schema_v7(&mut transaction, target, &v7_manifest, false)?;
            verify_writer_lease_v5_store_v8_successor(&mut transaction)?;
            let executable_count = apply_missing_entries(&mut transaction, 8)?;
            advance_compatibility_from_v7(&mut transaction, &v7_manifest, &manifest)?;
            MigrationApplyOutcome::Applied { executable_count }
        }
        InstalledManifestState::ExactV8LegacyPrefix => {
            verify_runtime_external_adoption_schema_v8(
                &mut transaction,
                target,
                &v8_legacy_manifest,
                false,
            )?;
            verify_writer_lease_v5_store_v8_successor(&mut transaction)?;
            let executable_count = apply_missing_entries(&mut transaction, 9)?;
            advance_compatibility_from_v8_legacy(&mut transaction, &v8_legacy_manifest, &manifest)?;
            MigrationApplyOutcome::Applied { executable_count }
        }
        InstalledManifestState::ExactV8Full => {
            verify_runtime_external_adoption_schema_v8(&mut transaction, target, &manifest, false)?;
            MigrationApplyOutcome::AlreadyCurrent
        }
    };

    preflight_connection(
        &mut transaction,
        target,
        DatabaseRole::Migrator,
        SetupOperation::Migration,
    )?;
    match installed {
        InstalledManifestState::ExactV5Prefix => {
            verify_runtime_foreman_schema_v6(
                &mut transaction,
                target,
                &v6_manifest,
                false,
                SchemaV6WriterProfile::V3Current,
            )?;
        }
        InstalledManifestState::ExactV6Prefix => {
            verify_runtime_submission_schema_v7(&mut transaction, target, &v7_manifest, false)?;
        }
        InstalledManifestState::ExactV7Prefix
        | InstalledManifestState::ExactV8LegacyPrefix
        | InstalledManifestState::ExactV8Full => {
            verify_runtime_external_adoption_schema_v8(&mut transaction, target, &manifest, false)?;
        }
        InstalledManifestState::Fresh
        | InstalledManifestState::ExactV1Prefix
        | InstalledManifestState::ExactV2Prefix
        | InstalledManifestState::ExactV3Prefix
        | InstalledManifestState::ExactV4Prefix => {
            let current_profile = classify_current_catalog_profile(&mut transaction, 5)?;
            verify_role_and_database_boundary(&mut transaction, current_profile, false)?;
        }
    }

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
#[allow(clippy::too_many_lines)]
pub fn verify_postgres_schema(
    client: &mut Client,
    target: &MigrationTarget,
    role: DatabaseRole,
) -> Result<PostgresSchemaEvidence, PostgresStoreSetupError> {
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
    let schema_version = u16::try_from(row_value::<i16>(
        &compatibility[0],
        0,
        PostgresStoreSetupErrorKind::CompatibilityMismatch,
    )?)
    .map_err(|_| {
        PostgresStoreSetupError::new(PostgresStoreSetupErrorKind::CompatibilityMismatch)
    })?;
    if schema_version > POSTGRES_SCHEMA_VERSION {
        return match classify_retained_history(&mut transaction)? {
            RetainedHistoryClassification::StrictFutureSuffix => Err(PostgresStoreSetupError::new(
                PostgresStoreSetupErrorKind::UnsupportedFutureSchema,
            )),
            RetainedHistoryClassification::ExactSupported
            | RetainedHistoryClassification::Corrupt => Err(history_error()),
        };
    }
    let evidence = if schema_version == POSTGRES_SCHEMA_VERSION {
        let manifest = verify_embedded_manifest()?;
        let database_uuid =
            verify_runtime_external_adoption_schema_v8(&mut transaction, target, &manifest, false)?;
        PostgresSchemaEvidence {
            database_uuid,
            manifest_sha256: manifest.manifest_sha256().clone(),
            schema_version,
            server_version_num: connection.server_version_num,
            role,
            bootstrap_admission: BootstrapAdmission::StoppedNoLeader,
        }
    } else if schema_version == 7 {
        let manifest = verify_v7_manifest_prefix()?;
        let database_uuid =
            verify_runtime_submission_schema_v7(&mut transaction, target, &manifest, false)?;
        PostgresSchemaEvidence {
            database_uuid,
            manifest_sha256: manifest.manifest_sha256().clone(),
            schema_version,
            server_version_num: connection.server_version_num,
            role,
            bootstrap_admission: BootstrapAdmission::StoppedNoLeader,
        }
    } else if schema_version == 6 {
        let manifest = verify_v6_manifest_prefix()?;
        let database_uuid = verify_runtime_foreman_schema_v6(
            &mut transaction,
            target,
            &manifest,
            false,
            SchemaV6WriterProfile::V3Current,
        )?;
        PostgresSchemaEvidence {
            database_uuid,
            manifest_sha256: manifest.manifest_sha256().clone(),
            schema_version,
            server_version_num: connection.server_version_num,
            role,
            bootstrap_admission: BootstrapAdmission::StoppedNoLeader,
        }
    } else if schema_version == 5 {
        let manifest = verify_v5_manifest_prefix()?;
        verify_catalog(
            &mut transaction,
            &manifest,
            target,
            role,
            connection.server_version_num,
        )?
    } else {
        return Err(PostgresStoreSetupError::new(
            PostgresStoreSetupErrorKind::CompatibilityMismatch,
        ));
    };
    preflight_connection(&mut transaction, target, role, SetupOperation::Verification)?;
    transaction.commit().map_err(|error| {
        map_postgres_error(&error, PostgresStoreSetupErrorKind::TransactionFailed)
    })?;
    Ok(evidence)
}

#[allow(clippy::too_many_lines)]
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
        5 => verify_v5_manifest_prefix()?,
        6 => verify_v6_manifest_prefix()?,
        7 => verify_v7_manifest_prefix()?,
        POSTGRES_SCHEMA_VERSION => verify_embedded_manifest()?,
        version if version > POSTGRES_SCHEMA_VERSION => {
            return match classify_retained_history(&mut transaction)? {
                RetainedHistoryClassification::StrictFutureSuffix => {
                    Err(PostgresStoreSetupError::new(
                        PostgresStoreSetupErrorKind::UnsupportedFutureSchema,
                    ))
                }
                RetainedHistoryClassification::ExactSupported
                | RetainedHistoryClassification::Corrupt => Err(history_error()),
            };
        }
        _ => {
            return Err(PostgresStoreSetupError::new(
                PostgresStoreSetupErrorKind::CompatibilityMismatch,
            ));
        }
    };
    if matches!(installed_schema_version, 6 | 7 | POSTGRES_SCHEMA_VERSION) {
        let database_uuid = if installed_schema_version == 6 {
            verify_runtime_foreman_schema_v6(
                &mut transaction,
                target,
                &manifest,
                true,
                SchemaV6WriterProfile::V3Current,
            )?
        } else if installed_schema_version == 7 {
            verify_runtime_submission_schema_v7(&mut transaction, target, &manifest, true)?
        } else {
            verify_runtime_external_adoption_schema_v8(&mut transaction, target, &manifest, true)?
        };
        preflight_connection(
            &mut transaction,
            target,
            DatabaseRole::Runtime,
            SetupOperation::Verification,
        )?;
        transaction.commit().map_err(|error| {
            map_postgres_error(&error, PostgresStoreSetupErrorKind::TransactionFailed)
        })?;
        return Ok(RuntimeStoreSchemaEvidence {
            database_uuid,
            global_manifest_sha256: manifest.manifest_sha256().clone(),
            global_schema_version: installed_schema_version,
            store_manifest_sha256: store_v2_manifest.manifest_sha256().clone(),
            store_schema_version: STORE_V2_SCHEMA_VERSION,
        });
    }
    let current_profile =
        classify_current_catalog_profile(&mut transaction, installed_schema_version)?;
    if matches!(
        current_profile,
        CatalogProfile::V3CodebaseMemoryV2WriterLeaseV2Bridge
            | CatalogProfile::V5CodebaseMemoryV2UpgradePending
            | CatalogProfile::V5CodebaseMemoryV2WriterLeaseV2BridgePending
            | CatalogProfile::V5CodebaseMemoryV3WriterLeaseV2BridgePending
    ) {
        return Err(PostgresStoreSetupError::new(
            PostgresStoreSetupErrorKind::CompatibilityMismatch,
        ));
    }
    let v3_prefix = installed_schema_version == 3;
    verify_schema_objects_with_contract(&mut transaction, current_profile, v3_prefix)?;
    let rows = read_history_rows(&mut transaction)?;
    let expected_history = match installed_schema_version {
        3 => &migration_manifest()[..4],
        5 => &migration_manifest()[..6],
        _ => migration_manifest(),
    };
    verify_history_rows(&rows, expected_history)?;
    verify_compatibility(&mut transaction, &manifest, current_profile)?;
    let database_uuid = read_database_identity(&mut transaction, target)?;
    verify_runtime_admission_present(&mut transaction)?;
    verify_roles_and_grants_with_contract(&mut transaction, current_profile, v3_prefix)?;
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SchemaV6WriterProfile {
    V3Current,
    V4Bridge,
}

const SCHEMA_V6_WRITER_V3_DANGEROUS_FUNCTION_COUNT: i64 = 35;
const SCHEMA_V6_WRITER_V4_BRIDGE_DANGEROUS_FUNCTION_COUNT: i64 = 28;
const SCHEMA_V7_WRITER_V4_DANGEROUS_FUNCTION_COUNT: i64 = 44;
const SCHEMA_V8_WRITER_V5_DANGEROUS_FUNCTION_COUNT: i64 = 47;
const MANAGED_FOREMAN_RUNTIME_FUNCTION_COUNT: i64 = 39;
const MANAGED_FOREMAN_EXTENSION_ID: &str = "lattice-postgres-foreman";
const MANAGED_FOREMAN_EXTENSION_SCHEMA_VERSION: i16 = 1;
const MANAGED_FOREMAN_EXTENSION_PATH: &str = "db/extensions/foreman-execution/v1.sql";
const MANAGED_FOREMAN_EXTENSION_SQL_BYTES: i64 = 349_546;
const MANAGED_FOREMAN_EXTENSION_SQL_SHA256: &str =
    "46e186d54b65fbd55f7d5f48c693707287e0d723bd10c3077412d484c19ead6e";
const MANAGED_FOREMAN_EXTENSION_MANIFEST_SHA256: &str =
    "2a487f0f32c45542d0ee02a37881f55466ca892f530967d95f661a27594279dd";
const MANAGED_FOREMAN_FUNCTION_CATALOG_DOMAIN: &[u8] =
    b"LATTICE_POSTGRES_FOREMAN_FUNCTION_CATALOG_V1\0";
const MANAGED_FOREMAN_TABLE_CATALOG_DOMAIN: &[u8] = b"LATTICE_POSTGRES_FOREMAN_TABLE_CATALOG_V2\0";
const MANAGED_FOREMAN_FUNCTION_CATALOG_SHA256: &str =
    "8d8dd263498cab48b1164bf456f5d3b314d575ee9a186460715beea02bc8bfec";
const MANAGED_FOREMAN_TABLE_CATALOG_SHA256: &str =
    "42f151dd9f52ba1e82a2aac392234f2b285c18e9bd71a00372f7c7b4a1237eb5";
const MANAGED_FOREMAN_STORE_V8_REBOUND_FUNCTION_CATALOG_SHA256: &str =
    "3874875a39369bd3e3e9238afbe5abd2cfc2cd4f29447d6013bcf59ffbb61bb0";
const MANAGED_FOREMAN_STORE_V8_REBOUND_TABLE_CATALOG_SHA256: &str =
    "28606d1ae0b3dce3f7f47f93dfde651fbe44c28d237ce9558bbbf6cad728078d";
const MANAGED_FOREMAN_CONTROL_INTERNAL_TRIGGER_COUNT: i64 = 12;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ManagedForemanStoreBinding {
    StoreV7Base,
    StoreV8Rebound,
}

#[derive(Debug)]
struct ManagedForemanPrincipalProfile {
    relation_oids: Vec<i64>,
    function_oids: Vec<i64>,
    binding: ManagedForemanStoreBinding,
}
const SCHEMA_V6_OWNED_CATALOG_SIGNATURES: [&str; 9] = [
    "dc5d05955070ecd8da9fc783cdb98091be7c960def04fd9e83cf77d7c5b00cf7",
    "5f68d85f2a5cc41d72984644284ceb73da1f6214dbd31f508fcdced961d2c517",
    "dab28ba9e1b7b4f4fea19e69bb3e72f9e8106b69d4403d5304b1243b8b7ce543",
    "2234eb282c4a53b632c529d307581d47eb2bf12827614878a76f00786c66f2a7",
    "4e5cb9af7070fddd2ba22e3042d2404b4a378536d519a3d9b3d739413e7e1c0d",
    "aa43280f9f62243291ecaf1b8aa00e425b52bd03c43cb94257e2d7d08e8ec276",
    "320fcc8f1d08ea2b465269b9c73964cea1e77a5f760c315c01cfb5392fa268fe",
    "30fec2dd985ad9e1244f31bdcd3ee2f074542b8551423377307aabdb867bb1e4",
    "093efdae2f43f0f5adfdb1296010e990fed1120e54401537939454a2952e7d8e",
];
const SCHEMA_V7_OWNED_CATALOG_SIGNATURES: [&str; 9] = [
    "a91e9d99aad9bd7d27c7a92f8ce398807966b598a8a8bcca2bd87e4912181b30",
    "f004466f320b20519f47cf79e4b19c2a139af248c886e693b4a4a3819d340f7a",
    "2c4d78fc5635ebbc257c50f2db045147dd06ba6a5982b70a0542f417dbfc78b2",
    "bc56d82c7f0412b4e5126b3e84fc4aa88701a611b22fb3f191055c62b507fc79",
    "9b13fe54389956deb6fc043611370445542d06161d881aa110305869b01fcd69",
    "493d397902d88db92ac4d517432e46eaaa661c38a0f91e852c2a2e1854cf047c",
    "01d71ae8cab8caaf07013872ff0e4d82fe4416c70dfb97793b0f64321c67da35",
    "fc6a34bccb67771c124262cff8e81acb65b465faf548ddcf4175fe2956b77092",
    "093efdae2f43f0f5adfdb1296010e990fed1120e54401537939454a2952e7d8e",
];
const SCHEMA_V8_OWNED_CATALOG_SIGNATURES: [&str; 9] = [
    "3c43887a313a90b32e20e53361e6dae623a5916526c41803a186a65b39c9795a",
    "5be6f26294388b0cbae0e85d2d99fa497487824d4ef72fe299fe1292c34028ca",
    "f4258cdf3531efef7658defd56fa24c7455af454fdbfccfe4ba7af8c98a314e9",
    "a3667ade9552b5345c2a56f826189f9d6a3409f64199953b8d6f05de12712176",
    "7189544020db41784243e992f841eedd70df5594f841d3076ecd4693e2688920",
    "eeee761ea5588445dfd0300b5a4d22144dac8d3bcd0e454b5bfbe491b7330554",
    "3117e0620ba44c5cded67b71ca3c9c9aa14bf0a4f120566c332b9f69bbba6ac5",
    "2369d531b85167613fdf006db26419b1cb092f7d33d4b325e359f428c30e3186",
    "093efdae2f43f0f5adfdb1296010e990fed1120e54401537939454a2952e7d8e",
];
const SCHEMA_V6_FORBIDDEN_SCHEMA_OBJECT_COUNTS: [i64; 10] = [61, 0, 0, 0, 0, 0, 0, 74, 0, 0];
const SCHEMA_V7_FORBIDDEN_SCHEMA_OBJECT_COUNTS: [i64; 10] = [71, 0, 0, 0, 0, 0, 0, 114, 0, 0];
const SCHEMA_V8_FORBIDDEN_SCHEMA_OBJECT_COUNTS: [i64; 10] = [74, 0, 0, 0, 0, 0, 0, 126, 0, 0];

fn verify_owned_catalog_signature_profile<C: GenericClient>(
    client: &mut C,
    expected: &[&str; 9],
) -> Result<(), PostgresStoreSetupError> {
    for (query, expected_signature) in [
        RELATION_SIGNATURE_SQL,
        COLUMN_SIGNATURE_SQL,
        CONSTRAINT_SIGNATURE_SQL,
        INDEX_SIGNATURE_SQL,
        FUNCTION_SIGNATURE_SQL,
        TYPE_CATALOG_SIGNATURE_SQL,
    ]
    .into_iter()
    .zip(&expected[..6])
    {
        if catalog_signature(client, query, PostgresStoreSetupErrorKind::CorruptCatalog)?
            != *expected_signature
        {
            return Err(catalog_error());
        }
    }
    for (query, expected_signature) in [
        TABLE_ACL_SIGNATURE_SQL,
        FUNCTION_ACL_SIGNATURE_SQL,
        SCHEMA_ACL_SIGNATURE_SQL,
    ]
    .into_iter()
    .zip(&expected[6..])
    {
        if catalog_signature(client, query, PostgresStoreSetupErrorKind::PermissionDenied)?
            != *expected_signature
        {
            return Err(permission_error());
        }
    }
    Ok(())
}

fn verify_schema_v6_v7_forbidden_object_profile<C: GenericClient>(
    client: &mut C,
    expected: &[i64; 10],
) -> Result<(), PostgresStoreSetupError> {
    if read_forbidden_schema_object_counts(client)? != *expected {
        return Err(catalog_error());
    }
    Ok(())
}

fn verify_schema_v8_forbidden_object_profile<C: GenericClient>(
    client: &mut C,
    has_managed_foreman: bool,
) -> Result<(), PostgresStoreSetupError> {
    let actual = read_forbidden_schema_object_counts(client)?;
    let mut expected = SCHEMA_V8_FORBIDDEN_SCHEMA_OBJECT_COUNTS;
    if has_managed_foreman {
        expected[7] += MANAGED_FOREMAN_CONTROL_INTERNAL_TRIGGER_COUNT;
    }
    if actual != expected {
        return Err(catalog_error());
    }
    Ok(())
}

fn verify_exact_default_acl_signature<C: GenericClient>(
    client: &mut C,
) -> Result<(), PostgresStoreSetupError> {
    if catalog_signature(
        client,
        DEFAULT_ACL_SIGNATURE_SQL,
        PostgresStoreSetupErrorKind::PermissionDenied,
    )? != EXPECTED_DEFAULT_ACL_SIGNATURE
    {
        return Err(permission_error());
    }
    Ok(())
}

#[allow(clippy::too_many_lines)]
fn verify_runtime_foreman_schema_v6<C: GenericClient>(
    client: &mut C,
    target: &MigrationTarget,
    manifest: &ManifestEvidence,
    runtime_active: bool,
    writer_profile: SchemaV6WriterProfile,
) -> Result<String, PostgresStoreSetupError> {
    let rows = read_history_rows(client)?;
    let retained = retained_history_rows(&rows)?;
    let compatibility = read_retained_schema_compatibility(client)?;
    match classify_retained_history_rows(&retained, &compatibility) {
        RetainedHistoryClassification::ExactSupported
            if compatibility.manifest_sha256 == manifest.manifest_sha256().as_str() => {}
        RetainedHistoryClassification::StrictFutureSuffix => {
            return Err(PostgresStoreSetupError::new(
                PostgresStoreSetupErrorKind::UnsupportedFutureSchema,
            ));
        }
        RetainedHistoryClassification::ExactSupported | RetainedHistoryClassification::Corrupt => {
            return Err(history_error());
        }
    }
    verify_owned_catalog_signature_profile(client, &SCHEMA_V6_OWNED_CATALOG_SIGNATURES)?;
    verify_schema_header_comments(client, "V6")?;
    verify_schema_v6_v7_forbidden_object_profile(
        client,
        &SCHEMA_V6_FORBIDDEN_SCHEMA_OBJECT_COUNTS,
    )?;
    verify_exact_default_acl_signature(client)?;
    verify_autonomy_receipt_profile(client)?;
    verify_forbidden_namespace_objects(client)?;
    verify_effective_default_privileges(client)?;
    let catalog = client
        .query_one(
            "SELECT \
                (SELECT count(*)::bigint FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace \
                  WHERE n.nspname='control' AND c.relkind='r'), \
                (SELECT count(*)::bigint FROM pg_proc p JOIN pg_namespace n ON n.oid=p.pronamespace \
                  WHERE n.nspname='control'), \
                (SELECT count(*) FILTER (WHERE pg_catalog.has_function_privilege(\
                    'lattice_runtime',p.oid,'EXECUTE'))::bigint \
                   FROM pg_proc p JOIN pg_namespace n ON n.oid=p.pronamespace \
                  WHERE n.nspname='control'), \
                (SELECT count(*)::bigint FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace \
                  WHERE n.nspname='control' AND c.relname='task_ledger_foreman_snapshots' \
                    AND c.relkind='r' AND pg_get_userbyid(c.relowner)='lattice_migrator'), \
                COALESCE(pg_catalog.has_table_privilege('lattice_runtime',\
                    'control.task_ledger_foreman_snapshots','SELECT,INSERT,UPDATE,DELETE,TRUNCATE,REFERENCES,TRIGGER'),false), \
                (SELECT count(*)::bigint FROM pg_proc p JOIN pg_namespace n ON n.oid=p.pronamespace \
                  WHERE n.nspname='control' AND p.proname IN (\
                    'task_ledger_record_foreman_snapshot_v1','task_ledger_read_foreman_snapshots_v1') \
                    AND pg_catalog.has_function_privilege('lattice_runtime',p.oid,'EXECUTE'))",
            &[],
        )
        .map_err(|error| map_postgres_error(&error, PostgresStoreSetupErrorKind::CorruptCatalog))?;
    let table_count = row_value::<i64>(&catalog, 0, PostgresStoreSetupErrorKind::CorruptCatalog)?;
    let retained_functions =
        row_value::<i64>(&catalog, 1, PostgresStoreSetupErrorKind::CorruptCatalog)?;
    let runtime_functions =
        row_value::<i64>(&catalog, 2, PostgresStoreSetupErrorKind::PermissionDenied)?;
    let foreman_table = row_value::<i64>(&catalog, 3, PostgresStoreSetupErrorKind::CorruptCatalog)?;
    let direct_table =
        row_value::<bool>(&catalog, 4, PostgresStoreSetupErrorKind::PermissionDenied)?;
    let foreman_runtime_functions =
        row_value::<i64>(&catalog, 5, PostgresStoreSetupErrorKind::PermissionDenied)?;
    if (
        table_count,
        retained_functions,
        runtime_functions,
        foreman_table,
        direct_table,
        foreman_runtime_functions,
    ) != (17, 49, 21, 1, false, 2)
    {
        return Err(catalog_error());
    }

    let writer = client
        .query_one(
            "SELECT \
                (SELECT count(*)::bigint FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace \
                  WHERE n.nspname='writer_lease' AND c.relkind='r'), \
                (SELECT count(*)::bigint FROM pg_proc p JOIN pg_namespace n ON n.oid=p.pronamespace \
                  WHERE n.nspname='writer_lease'), \
                (SELECT count(*) FILTER (WHERE pg_catalog.has_function_privilege(\
                    'lattice_runtime',p.oid,'EXECUTE'))::bigint \
                   FROM pg_proc p JOIN pg_namespace n ON n.oid=p.pronamespace \
                  WHERE n.nspname='writer_lease'), \
                COALESCE(pg_catalog.has_schema_privilege('lattice_runtime','writer_lease','USAGE'),false)",
            &[],
        )
        .map_err(|error| map_postgres_error(&error, PostgresStoreSetupErrorKind::CorruptCatalog))?;
    let writer_catalog = (
        row_value::<i64>(&writer, 0, PostgresStoreSetupErrorKind::CorruptCatalog)?,
        row_value::<i64>(&writer, 1, PostgresStoreSetupErrorKind::CorruptCatalog)?,
        row_value::<i64>(&writer, 2, PostgresStoreSetupErrorKind::PermissionDenied)?,
        row_value::<bool>(&writer, 3, PostgresStoreSetupErrorKind::PermissionDenied)?,
    );
    let expected_dangerous_functions = match writer_profile {
        SchemaV6WriterProfile::V3Current if writer_catalog == (5, 12, 7, true) => {
            verify_writer_lease_exact_catalog_profile(
                client,
                &WRITER_LEASE_V3_CURRENT_CATALOG_SIGNATURES,
            )?;
            verify_writer_lease_v3_functions(client, true)?;
            verify_writer_lease_acl_closure(client, 5, true)?;
            SCHEMA_V6_WRITER_V3_DANGEROUS_FUNCTION_COUNT
        }
        SchemaV6WriterProfile::V4Bridge if writer_catalog == (5, 15, 0, false) => {
            verify_writer_lease_exact_catalog_profile(
                client,
                &WRITER_LEASE_V4_BRIDGE_CATALOG_SIGNATURES,
            )?;
            verify_writer_lease_v4_functions(client, false)?;
            verify_writer_lease_acl_closure(client, 15, false)?;
            SCHEMA_V6_WRITER_V4_BRIDGE_DANGEROUS_FUNCTION_COUNT
        }
        SchemaV6WriterProfile::V3Current | SchemaV6WriterProfile::V4Bridge => {
            return Err(catalog_error());
        }
    };

    let migration = &migration_manifest()[6];
    let candidate = ForemanSchemaV6Candidate::from_migration_bytes(
        migration.ordinal(),
        migration.id(),
        migration.path(),
        migration.schema_version(),
        migration.reader_compatibility(),
        migration.writer_compatibility(),
        FOREMAN_COORDINATION_STREAM_IDENTITY,
        FOREMAN_COORDINATION_EVENT_IDENTITY,
        migration.bytes(),
    )
    .map_err(|_| catalog_error())?;
    let _verified_profile = verify_foreman_schema_v6_profile(
        &candidate,
        &ForemanSchemaV6CatalogAcl::exact_foreman_coordination(),
        WriterLeaseV3Profile::Current,
    )
    .map_err(|_| catalog_error())?;
    if runtime_active {
        verify_runtime_admission_present(client)?;
    } else {
        verify_stopped_admission(client)?;
    }
    verify_exact_principal_database_boundary(client, expected_dangerous_functions, true, None)?;
    read_database_identity(client, target)
}

fn verify_v7_ingress_ambiguity_profile<C: GenericClient>(
    client: &mut C,
) -> Result<(), PostgresStoreSetupError> {
    let profiles = [
        (
            "RELATION",
            V7_AMBIGUITY_RELATION_SIGNATURE_SQL,
            V7_AMBIGUITY_RELATION_SIGNATURE,
            PostgresStoreSetupErrorKind::CorruptCatalog,
        ),
        (
            "COLUMN",
            V7_AMBIGUITY_COLUMN_SIGNATURE_SQL,
            V7_AMBIGUITY_COLUMN_SIGNATURE,
            PostgresStoreSetupErrorKind::CorruptCatalog,
        ),
        (
            "CONSTRAINT",
            V7_AMBIGUITY_CONSTRAINT_SIGNATURE_SQL,
            V7_AMBIGUITY_CONSTRAINT_SIGNATURE,
            PostgresStoreSetupErrorKind::CorruptCatalog,
        ),
        (
            "INDEX",
            V7_AMBIGUITY_INDEX_SIGNATURE_SQL,
            V7_AMBIGUITY_INDEX_SIGNATURE,
            PostgresStoreSetupErrorKind::CorruptCatalog,
        ),
        (
            "TABLE_ACL",
            V7_AMBIGUITY_TABLE_ACL_SIGNATURE_SQL,
            V7_AMBIGUITY_TABLE_ACL_SIGNATURE,
            PostgresStoreSetupErrorKind::PermissionDenied,
        ),
        (
            "FUNCTION",
            V7_INGRESS_FUNCTION_SIGNATURE_SQL,
            V7_INGRESS_FUNCTION_SIGNATURE,
            PostgresStoreSetupErrorKind::CorruptCatalog,
        ),
        (
            "FUNCTION_ACL",
            V7_INGRESS_FUNCTION_ACL_SIGNATURE_SQL,
            V7_INGRESS_FUNCTION_ACL_SIGNATURE,
            PostgresStoreSetupErrorKind::PermissionDenied,
        ),
    ];
    let mut mismatch = None;
    for (_, query, expected, error_kind) in profiles {
        let actual = catalog_signature(client, query, error_kind)?;
        if actual != expected && mismatch.is_none() {
            mismatch = Some(error_kind);
        }
    }
    if let Some(error_kind) = mismatch {
        return Err(PostgresStoreSetupError::new(error_kind));
    }

    let product = verify_optional_control_product_extension(client)?.is_some();
    let closure_sql = if product {
        "SELECT control_product.task_ingress_historical_closure_v1()"
    } else {
        "SELECT control.task_ingress_historical_closure_v1()"
    };
    let closure = client.query_one(closure_sql, &[]).map_err(|error| {
        map_postgres_error(&error, PostgresStoreSetupErrorKind::PermissionDenied)
    })?;
    if !row_value::<bool>(&closure, 0, PostgresStoreSetupErrorKind::HistoryMismatch)? {
        return Err(history_error());
    }
    Ok(())
}

const MANAGED_FOREMAN_FUNCTION_CATALOG_SQL: &str = r"
    WITH function_profile AS (
        SELECT 1 AS kind,p.proname::text AS function_name,
               pg_catalog.pg_get_function_identity_arguments(p.oid)::text
                   AS identity_arguments,''::text AS item_key,
               pg_catalog.json_build_array(
                   'FUNCTION_PROFILE',p.proname,
                   pg_catalog.pg_get_function_identity_arguments(p.oid),
                   pg_catalog.pg_get_function_result(p.oid),
                   pg_catalog.pg_get_functiondef(p.oid),owner.rolname,
                   pg_catalog.has_function_privilege(
                       'lattice_runtime',p.oid,'EXECUTE'),
                   EXISTS (SELECT 1 FROM pg_catalog.aclexplode(
                       COALESCE(p.proacl,pg_catalog.acldefault('f',p.proowner)))
                       AS public_acl WHERE public_acl.grantee=0
                       AND public_acl.privilege_type='EXECUTE')
               )::text AS value
          FROM pg_catalog.pg_proc AS p
          JOIN pg_catalog.pg_namespace AS n ON n.oid=p.pronamespace
          JOIN pg_catalog.pg_roles AS owner ON owner.oid=p.proowner
         WHERE n.nspname='foreman_execution'
        UNION ALL
        SELECT 2,p.proname::text,
               pg_catalog.pg_get_function_identity_arguments(p.oid)::text,
               pg_catalog.json_build_array(
                   CASE WHEN acl.grantee=0 THEN 'PUBLIC'
                        ELSE grantee.rolname END,grantor.rolname,
                   acl.privilege_type,acl.is_grantable)::text,
               pg_catalog.json_build_array(
                   'FUNCTION_ACL',p.proname,
                   pg_catalog.pg_get_function_identity_arguments(p.oid),
                   CASE WHEN acl.grantee=0 THEN 'PUBLIC'
                        ELSE grantee.rolname END,grantor.rolname,
                   acl.privilege_type,acl.is_grantable)::text
          FROM pg_catalog.pg_proc AS p
          JOIN pg_catalog.pg_namespace AS n ON n.oid=p.pronamespace
          CROSS JOIN LATERAL pg_catalog.aclexplode(
               COALESCE(p.proacl,pg_catalog.acldefault('f',p.proowner))) AS acl
          LEFT JOIN pg_catalog.pg_roles AS grantee ON grantee.oid=acl.grantee
          JOIN pg_catalog.pg_roles AS grantor ON grantor.oid=acl.grantor
         WHERE n.nspname='foreman_execution'
    )
    SELECT value FROM function_profile
    ORDER BY kind,function_name,identity_arguments,item_key
";

const MANAGED_FOREMAN_TABLE_CATALOG_SQL: &str = r"
    WITH profile AS (
        SELECT 0 AS kind,n.nspname::text AS relation_name,''::text AS item_key,
               pg_catalog.json_build_array(
                   'SCHEMA_PROFILE',n.nspname,schema_owner.rolname,
                   pg_catalog.obj_description(n.oid,'pg_namespace'))::text AS value
          FROM pg_catalog.pg_namespace AS n
          JOIN pg_catalog.pg_roles AS schema_owner ON schema_owner.oid = n.nspowner
         WHERE n.nspname='foreman_execution'
        UNION ALL
        SELECT 1,n.nspname::text,
               pg_catalog.json_build_array(
                   CASE WHEN acl.grantee=0 THEN 'PUBLIC'
                        ELSE grantee.rolname END,grantor.rolname,
                   acl.privilege_type,acl.is_grantable)::text,
               pg_catalog.json_build_array(
                   'SCHEMA_ACL',n.nspname,
                   CASE WHEN acl.grantee=0 THEN 'PUBLIC'
                        ELSE grantee.rolname END,grantor.rolname,
                   acl.privilege_type,acl.is_grantable)::text
          FROM pg_catalog.pg_namespace AS n
          CROSS JOIN LATERAL pg_catalog.aclexplode(
                COALESCE(n.nspacl, pg_catalog.acldefault('n',n.nspowner))) AS acl
          LEFT JOIN pg_catalog.pg_roles AS grantee ON grantee.oid=acl.grantee
          JOIN pg_catalog.pg_roles AS grantor ON grantor.oid=acl.grantor
         WHERE n.nspname='foreman_execution'
        UNION ALL
        SELECT 2,c.relname::text,''::text,
               pg_catalog.json_build_array(
                   'TABLE',c.relname,owner.rolname,c.relrowsecurity,
                   c.relforcerowsecurity,c.relreplident,c.relpersistence,
                   COALESCE(pg_catalog.array_to_string(c.reloptions,','),'<NULL>'),
                   COALESCE(pg_catalog.array_to_string(toast.reloptions,','),'<NULL>'),
                   pg_catalog.obj_description(c.oid,'pg_class'))::text
          FROM pg_catalog.pg_class AS c
          JOIN pg_catalog.pg_namespace AS n ON n.oid=c.relnamespace
          JOIN pg_catalog.pg_roles AS owner ON owner.oid=c.relowner
          LEFT JOIN pg_catalog.pg_class AS toast ON toast.oid=c.reltoastrelid
         WHERE n.nspname='foreman_execution' AND c.relkind='r'
        UNION ALL
        SELECT 3,c.relname::text,pg_catalog.lpad(a.attnum::text,5,'0'),
               pg_catalog.json_build_array(
                   'COLUMN',c.relname,a.attnum,a.attname,
                   pg_catalog.format_type(a.atttypid,a.atttypmod),
                   coll_ns.nspname,coll.collname,a.attnotnull,a.attisdropped,a.attidentity,
                   a.attgenerated,a.attstorage,a.attcompression,a.attstattarget,
                   COALESCE(pg_catalog.array_to_string(a.attoptions,','),'<NULL>'),
                   pg_catalog.pg_get_expr(d.adbin,d.adrelid),
                   pg_catalog.col_description(c.oid,a.attnum))::text
          FROM pg_catalog.pg_class AS c
          JOIN pg_catalog.pg_namespace AS n ON n.oid=c.relnamespace
          JOIN pg_catalog.pg_attribute AS a
            ON a.attrelid=c.oid AND a.attnum>0
          LEFT JOIN pg_catalog.pg_attrdef AS d
            ON d.adrelid=c.oid AND d.adnum=a.attnum
          LEFT JOIN pg_catalog.pg_collation AS coll ON coll.oid=a.attcollation
          LEFT JOIN pg_catalog.pg_namespace AS coll_ns ON coll_ns.oid=coll.collnamespace
         WHERE n.nspname='foreman_execution' AND c.relkind='r'
        UNION ALL
        SELECT 4,c.relname::text,k.conname::text,
               pg_catalog.json_build_array(
                   'CONSTRAINT',c.relname,k.conname,k.contype,
                   pg_catalog.pg_get_constraintdef(k.oid,false),
                   pg_catalog.obj_description(k.oid,'pg_constraint'))::text
          FROM pg_catalog.pg_constraint AS k
          JOIN pg_catalog.pg_class AS c ON c.oid=k.conrelid
          JOIN pg_catalog.pg_namespace AS n ON n.oid=c.relnamespace
         WHERE n.nspname='foreman_execution'
        UNION ALL
        SELECT 5,t.relname::text,i.relname::text,
               pg_catalog.json_build_array(
                   'INDEX',t.relname,i.relname,
                   pg_catalog.pg_get_indexdef(i.oid),
                   COALESCE(pg_catalog.array_to_string(i.reloptions,','),'<NULL>'),
                   pg_catalog.obj_description(i.oid,'pg_class'))::text
          FROM pg_catalog.pg_index AS x
          JOIN pg_catalog.pg_class AS i ON i.oid=x.indexrelid
          JOIN pg_catalog.pg_class AS t ON t.oid=x.indrelid
          JOIN pg_catalog.pg_namespace AS n ON n.oid=t.relnamespace
         WHERE n.nspname='foreman_execution'
        UNION ALL
        SELECT 6,c.relname::text,
               pg_catalog.json_build_array(
                   CASE WHEN acl.grantee=0 THEN 'PUBLIC'
                        ELSE grantee.rolname END,grantor.rolname,
                   acl.privilege_type,acl.is_grantable)::text,
               pg_catalog.json_build_array(
                   'TABLE_ACL',c.relname,
                   CASE WHEN acl.grantee=0 THEN 'PUBLIC'
                        ELSE grantee.rolname END,grantor.rolname,
                   acl.privilege_type,acl.is_grantable)::text
          FROM pg_catalog.pg_class AS c
          JOIN pg_catalog.pg_namespace AS n ON n.oid=c.relnamespace
          CROSS JOIN LATERAL pg_catalog.aclexplode(
               COALESCE(c.relacl,pg_catalog.acldefault('r',c.relowner))) AS acl
          LEFT JOIN pg_catalog.pg_roles AS grantee ON grantee.oid=acl.grantee
          JOIN pg_catalog.pg_roles AS grantor ON grantor.oid=acl.grantor
         WHERE n.nspname='foreman_execution' AND c.relkind='r'
        UNION ALL
        SELECT 7,c.relname::text,
               pg_catalog.lpad(a.attnum::text,5,'0') || ':' ||
               pg_catalog.json_build_array(
                   CASE WHEN acl.grantee=0 THEN 'PUBLIC'
                        ELSE grantee.rolname END,grantor.rolname,
                   acl.privilege_type,acl.is_grantable)::text,
               pg_catalog.json_build_array(
                   'TABLE_COLUMN_ACL',c.relname,a.attnum,a.attname,
                   CASE WHEN acl.grantee=0 THEN 'PUBLIC'
                        ELSE grantee.rolname END,grantor.rolname,
                   acl.privilege_type,acl.is_grantable)::text
          FROM pg_catalog.pg_class AS c
          JOIN pg_catalog.pg_namespace AS n ON n.oid=c.relnamespace
          JOIN pg_catalog.pg_attribute AS a
            ON a.attrelid=c.oid AND a.attnum>0 AND NOT a.attisdropped
          CROSS JOIN LATERAL pg_catalog.aclexplode(a.attacl) AS acl
          LEFT JOIN pg_catalog.pg_roles AS grantee ON grantee.oid=acl.grantee
          JOIN pg_catalog.pg_roles AS grantor ON grantor.oid=acl.grantor
         WHERE n.nspname='foreman_execution' AND c.relkind='r'
    )
    SELECT value FROM profile ORDER BY kind,relation_name,item_key
";

fn managed_foreman_catalog_digest<C: GenericClient>(
    client: &mut C,
    query: &str,
    domain: &[u8],
) -> Result<String, PostgresStoreSetupError> {
    let rows = client
        .query(query, &[])
        .map_err(|error| map_postgres_error(&error, PostgresStoreSetupErrorKind::CorruptCatalog))?;
    let mut hasher = Sha256::new();
    hasher.update(domain);
    for row in &rows {
        let value = row_value::<String>(row, 0, PostgresStoreSetupErrorKind::CorruptCatalog)?;
        hasher.update(
            u64::try_from(value.len())
                .map_err(|_| catalog_error())?
                .to_be_bytes(),
        );
        hasher.update(value.as_bytes());
    }
    Ok(hex_digest(hasher.finalize().as_ref()))
}

#[allow(clippy::too_many_lines)]
fn verify_managed_foreman_identity_and_history<C: GenericClient>(
    client: &mut C,
    target: &MigrationTarget,
    binding: ManagedForemanStoreBinding,
) -> Result<(), PostgresStoreSetupError> {
    let (global_version, global_manifest, expected_ledger_rows) = match binding {
        ManagedForemanStoreBinding::StoreV7Base => (7_i16, CURRENT_V7_MANIFEST_SHA256, 1_usize),
        ManagedForemanStoreBinding::StoreV8Rebound => (8_i16, CURRENT_V8_MANIFEST_SHA256, 2_usize),
    };
    let identities = client
        .query(
            "SELECT extension_id::text,extension_schema_version,extension_path::text, \
                    extension_sql_bytes,pg_catalog.btrim(extension_sql_sha256)::text, \
                    pg_catalog.btrim(extension_manifest_sha256)::text,database_name::text, \
                    database_uuid::text,pg_catalog.btrim(database_identity_sha256)::text, \
                    global_schema_version,pg_catalog.btrim(global_manifest_sha256)::text \
               FROM ONLY foreman_execution.extension_identity WHERE singleton",
            &[],
        )
        .map_err(|error| map_postgres_error(&error, PostgresStoreSetupErrorKind::CorruptCatalog))?;
    if identities.len() != 1 {
        return Err(catalog_error());
    }
    let identity = &identities[0];
    if row_value::<String>(identity, 0, PostgresStoreSetupErrorKind::CorruptCatalog)?
        != MANAGED_FOREMAN_EXTENSION_ID
        || row_value::<i16>(identity, 1, PostgresStoreSetupErrorKind::CorruptCatalog)?
            != MANAGED_FOREMAN_EXTENSION_SCHEMA_VERSION
        || row_value::<String>(identity, 2, PostgresStoreSetupErrorKind::CorruptCatalog)?
            != MANAGED_FOREMAN_EXTENSION_PATH
        || row_value::<i64>(identity, 3, PostgresStoreSetupErrorKind::CorruptCatalog)?
            != MANAGED_FOREMAN_EXTENSION_SQL_BYTES
        || row_value::<String>(identity, 4, PostgresStoreSetupErrorKind::CorruptCatalog)?
            != MANAGED_FOREMAN_EXTENSION_SQL_SHA256
        || row_value::<String>(identity, 5, PostgresStoreSetupErrorKind::CorruptCatalog)?
            != MANAGED_FOREMAN_EXTENSION_MANIFEST_SHA256
        || row_value::<String>(identity, 6, PostgresStoreSetupErrorKind::CorruptCatalog)?
            != target.database_name()
        || row_value::<String>(identity, 7, PostgresStoreSetupErrorKind::CorruptCatalog)?
            != target.expected_database_uuid()
        || row_value::<String>(identity, 8, PostgresStoreSetupErrorKind::CorruptCatalog)?
            != target.expected_database_identity_sha256().as_str()
        || row_value::<i16>(identity, 9, PostgresStoreSetupErrorKind::CorruptCatalog)?
            != global_version
        || row_value::<String>(identity, 10, PostgresStoreSetupErrorKind::CorruptCatalog)?
            != global_manifest
    {
        return Err(catalog_error());
    }

    let ledger = client
        .query(
            "SELECT ledger_ordinal,extension_id::text,extension_schema_version, \
                    pg_catalog.btrim(extension_sql_sha256)::text, \
                    pg_catalog.btrim(extension_manifest_sha256)::text,database_uuid::text, \
                    pg_catalog.btrim(database_identity_sha256)::text,global_schema_version, \
                    pg_catalog.btrim(global_manifest_sha256)::text,event_kind::text \
               FROM ONLY foreman_execution.extension_ledger ORDER BY ledger_ordinal",
            &[],
        )
        .map_err(|error| map_postgres_error(&error, PostgresStoreSetupErrorKind::CorruptCatalog))?;
    if ledger.len() != expected_ledger_rows {
        return Err(catalog_error());
    }
    for (index, row) in ledger.iter().enumerate() {
        let (expected_ordinal, expected_global_version, expected_global_manifest, expected_event) =
            if index == 0 {
                (1_i16, 7_i16, CURRENT_V7_MANIFEST_SHA256, "INSTALLED")
            } else {
                (2_i16, 8_i16, CURRENT_V8_MANIFEST_SHA256, "REBOUND")
            };
        if row_value::<i16>(row, 0, PostgresStoreSetupErrorKind::CorruptCatalog)?
            != expected_ordinal
            || row_value::<String>(row, 1, PostgresStoreSetupErrorKind::CorruptCatalog)?
                != MANAGED_FOREMAN_EXTENSION_ID
            || row_value::<i16>(row, 2, PostgresStoreSetupErrorKind::CorruptCatalog)?
                != MANAGED_FOREMAN_EXTENSION_SCHEMA_VERSION
            || row_value::<String>(row, 3, PostgresStoreSetupErrorKind::CorruptCatalog)?
                != MANAGED_FOREMAN_EXTENSION_SQL_SHA256
            || row_value::<String>(row, 4, PostgresStoreSetupErrorKind::CorruptCatalog)?
                != MANAGED_FOREMAN_EXTENSION_MANIFEST_SHA256
            || row_value::<String>(row, 5, PostgresStoreSetupErrorKind::CorruptCatalog)?
                != target.expected_database_uuid()
            || row_value::<String>(row, 6, PostgresStoreSetupErrorKind::CorruptCatalog)?
                != target.expected_database_identity_sha256().as_str()
            || row_value::<i16>(row, 7, PostgresStoreSetupErrorKind::CorruptCatalog)?
                != expected_global_version
            || row_value::<String>(row, 8, PostgresStoreSetupErrorKind::CorruptCatalog)?
                != expected_global_manifest
            || row_value::<String>(row, 9, PostgresStoreSetupErrorKind::CorruptCatalog)?
                != expected_event
        {
            return Err(catalog_error());
        }
    }
    Ok(())
}

fn verify_managed_foreman_reader_identity<C: GenericClient>(
    client: &mut C,
    target: &MigrationTarget,
    binding: ManagedForemanStoreBinding,
) -> Result<(), PostgresStoreSetupError> {
    let (global_version, global_manifest) = match binding {
        ManagedForemanStoreBinding::StoreV7Base => (7_i16, CURRENT_V7_MANIFEST_SHA256),
        ManagedForemanStoreBinding::StoreV8Rebound => (8_i16, CURRENT_V8_MANIFEST_SHA256),
    };
    let identities = client
        .query(
            "SELECT extension_id,extension_schema_version,extension_path,extension_sql_bytes, \
                    extension_sql_sha256,extension_manifest_sha256,database_name,database_uuid, \
                    database_identity_sha256,global_schema_version,global_manifest_sha256 \
               FROM foreman_execution.read_extension_identity_v1()",
            &[],
        )
        .map_err(|error| map_postgres_error(&error, PostgresStoreSetupErrorKind::CorruptCatalog))?;
    if identities.len() != 1 {
        return Err(catalog_error());
    }
    let identity = &identities[0];
    if row_value::<String>(identity, 0, PostgresStoreSetupErrorKind::CorruptCatalog)?
        != MANAGED_FOREMAN_EXTENSION_ID
        || row_value::<i16>(identity, 1, PostgresStoreSetupErrorKind::CorruptCatalog)?
            != MANAGED_FOREMAN_EXTENSION_SCHEMA_VERSION
        || row_value::<String>(identity, 2, PostgresStoreSetupErrorKind::CorruptCatalog)?
            != MANAGED_FOREMAN_EXTENSION_PATH
        || row_value::<i64>(identity, 3, PostgresStoreSetupErrorKind::CorruptCatalog)?
            != MANAGED_FOREMAN_EXTENSION_SQL_BYTES
        || row_value::<String>(identity, 4, PostgresStoreSetupErrorKind::CorruptCatalog)?
            != MANAGED_FOREMAN_EXTENSION_SQL_SHA256
        || row_value::<String>(identity, 5, PostgresStoreSetupErrorKind::CorruptCatalog)?
            != MANAGED_FOREMAN_EXTENSION_MANIFEST_SHA256
        || row_value::<String>(identity, 6, PostgresStoreSetupErrorKind::CorruptCatalog)?
            != target.database_name()
        || row_value::<String>(identity, 7, PostgresStoreSetupErrorKind::CorruptCatalog)?
            != target.expected_database_uuid()
        || row_value::<String>(identity, 8, PostgresStoreSetupErrorKind::CorruptCatalog)?
            != target.expected_database_identity_sha256().as_str()
        || row_value::<i16>(identity, 9, PostgresStoreSetupErrorKind::CorruptCatalog)?
            != global_version
        || row_value::<String>(identity, 10, PostgresStoreSetupErrorKind::CorruptCatalog)?
            != global_manifest
    {
        return Err(catalog_error());
    }
    Ok(())
}

#[allow(clippy::too_many_lines)]
fn verify_optional_managed_foreman_extension<C: GenericClient>(
    client: &mut C,
    target: &MigrationTarget,
) -> Result<Option<ManagedForemanPrincipalProfile>, PostgresStoreSetupError> {
    let presence = client
        .query_one(
            "SELECT pg_catalog.to_regnamespace('foreman_execution') IS NOT NULL, \
                    (SELECT pg_catalog.count(*) FROM pg_catalog.pg_class c \
                      JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace \
                     WHERE n.nspname='foreman_execution'), \
                    (SELECT pg_catalog.count(*) FROM pg_catalog.pg_proc p \
                      JOIN pg_catalog.pg_namespace n ON n.oid=p.pronamespace \
                     WHERE n.nspname='foreman_execution'), \
                    (SELECT pg_catalog.count(*) FROM pg_catalog.pg_class c \
                      JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace \
                     WHERE n.nspname='foreman_execution' \
                       AND c.relname='extension_identity' AND c.relkind='r')",
            &[],
        )
        .map_err(|error| map_postgres_error(&error, PostgresStoreSetupErrorKind::CorruptCatalog))?;
    let schema_exists =
        row_value::<bool>(&presence, 0, PostgresStoreSetupErrorKind::CorruptCatalog)?;
    let relations = row_value::<i64>(&presence, 1, PostgresStoreSetupErrorKind::CorruptCatalog)?;
    let functions = row_value::<i64>(&presence, 2, PostgresStoreSetupErrorKind::CorruptCatalog)?;
    let identity_tables =
        row_value::<i64>(&presence, 3, PostgresStoreSetupErrorKind::CorruptCatalog)?;
    if !schema_exists && relations == 0 && functions == 0 && identity_tables == 0 {
        return Ok(None);
    }
    if !schema_exists || relations != 58 || functions != 43 || identity_tables != 1 {
        return Err(catalog_error());
    }

    // Hash every function definition and every modeled table/index/ACL row
    // before invoking the extension-owned SECURITY DEFINER identity reader.
    let function_digest = managed_foreman_catalog_digest(
        client,
        MANAGED_FOREMAN_FUNCTION_CATALOG_SQL,
        MANAGED_FOREMAN_FUNCTION_CATALOG_DOMAIN,
    )?;
    let table_digest = managed_foreman_catalog_digest(
        client,
        MANAGED_FOREMAN_TABLE_CATALOG_SQL,
        MANAGED_FOREMAN_TABLE_CATALOG_DOMAIN,
    )?;
    let binding = match (function_digest.as_str(), table_digest.as_str()) {
        (MANAGED_FOREMAN_FUNCTION_CATALOG_SHA256, MANAGED_FOREMAN_TABLE_CATALOG_SHA256) => {
            ManagedForemanStoreBinding::StoreV7Base
        }
        (
            MANAGED_FOREMAN_STORE_V8_REBOUND_FUNCTION_CATALOG_SHA256,
            MANAGED_FOREMAN_STORE_V8_REBOUND_TABLE_CATALOG_SHA256,
        ) => ManagedForemanStoreBinding::StoreV8Rebound,
        _ => return Err(catalog_error()),
    };

    let current_role = client
        .query_one("SELECT current_user::text", &[])
        .map_err(|error| {
            map_postgres_error(&error, PostgresStoreSetupErrorKind::PermissionDenied)
        })?;
    let current_role = row_value::<String>(
        &current_role,
        0,
        PostgresStoreSetupErrorKind::PermissionDenied,
    )?;
    if !matches!(
        current_role.as_str(),
        "lattice_migrator" | "lattice_runtime" | "lattice_guardian" | "lattice_readonly"
    ) {
        return Err(permission_error());
    }
    // The V7 reader is deliberately unusable during the Store-V8 transition,
    // so the migrator classifies raw identity/history after pinning the entire
    // companion catalog. Ordinary roles have no raw table grants and use the
    // correspondingly pinned SECURITY DEFINER reader instead.
    if current_role == "lattice_migrator" {
        verify_managed_foreman_identity_and_history(client, target, binding)?;
    } else {
        verify_managed_foreman_reader_identity(client, target, binding)?;
    }

    let shape = client
        .query_one(
            "SELECT \
                (SELECT pg_catalog.count(*) FROM pg_catalog.pg_class c \
                  JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace \
                 WHERE n.nspname='foreman_execution' AND c.relkind='r'), \
                (SELECT pg_catalog.count(*) FROM pg_catalog.pg_class c \
                  JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace \
                 WHERE n.nspname='foreman_execution' AND c.relkind='i'), \
                (SELECT pg_catalog.count(*) FROM pg_catalog.pg_proc p \
                  JOIN pg_catalog.pg_namespace n ON n.oid=p.pronamespace \
                 WHERE n.nspname='foreman_execution'), \
                (SELECT pg_catalog.count(*) FROM pg_catalog.pg_class c \
                  JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace \
                  JOIN pg_catalog.pg_roles r ON r.oid=c.relowner \
                 WHERE n.nspname='foreman_execution' AND c.relkind='r' \
                   AND r.rolname='lattice_migrator'), \
                (SELECT pg_catalog.count(*) FROM pg_catalog.pg_proc p \
                  JOIN pg_catalog.pg_namespace n ON n.oid=p.pronamespace \
                  JOIN pg_catalog.pg_roles r ON r.oid=p.proowner \
                 WHERE n.nspname='foreman_execution' \
                   AND r.rolname='lattice_migrator' AND p.prosecdef \
                   AND p.proconfig=ARRAY['search_path=pg_catalog']::text[]), \
                (SELECT pg_catalog.count(*) FROM pg_catalog.pg_class c \
                  JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace \
                 WHERE n.nspname='foreman_execution' AND c.relkind='r' \
                   AND pg_catalog.has_table_privilege('lattice_runtime',c.oid, \
                     'SELECT,INSERT,UPDATE,DELETE,TRUNCATE,REFERENCES,TRIGGER')), \
                (SELECT pg_catalog.count(*) FROM pg_catalog.pg_proc p \
                  JOIN pg_catalog.pg_namespace n ON n.oid=p.pronamespace \
                 WHERE n.nspname='foreman_execution' \
                   AND pg_catalog.has_function_privilege('lattice_runtime',p.oid,'EXECUTE')), \
                (SELECT EXISTS (SELECT 1 FROM pg_catalog.pg_namespace n \
                  CROSS JOIN LATERAL pg_catalog.aclexplode( \
                    COALESCE(n.nspacl,pg_catalog.acldefault('n',n.nspowner))) a \
                 WHERE n.nspname='foreman_execution' AND a.grantee=0 \
                   AND a.privilege_type='USAGE')), \
                pg_catalog.has_schema_privilege('lattice_runtime','foreman_execution','USAGE'), \
                pg_catalog.has_schema_privilege('lattice_runtime','foreman_execution','CREATE'), \
                (SELECT pg_catalog.count(*) FROM pg_catalog.pg_trigger t \
                  JOIN pg_catalog.pg_class c ON c.oid=t.tgrelid \
                  JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace \
                 WHERE n.nspname='foreman_execution' AND NOT t.tgisinternal), \
                (SELECT pg_catalog.count(*) FROM pg_catalog.pg_trigger t \
                  JOIN pg_catalog.pg_class c ON c.oid=t.tgrelid \
                  JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace \
                 WHERE n.nspname='foreman_execution' AND t.tgisinternal), \
                (SELECT pg_catalog.count(*) FROM pg_catalog.pg_trigger t \
                  JOIN pg_catalog.pg_class c ON c.oid=t.tgrelid \
                  JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace \
                 WHERE n.nspname='foreman_execution' AND t.tgisinternal \
                   AND t.tgenabled='O' AND t.tgconstraint<>0), \
                (SELECT pg_catalog.count(*) FROM pg_catalog.pg_trigger t \
                  JOIN pg_catalog.pg_class c ON c.oid=t.tgrelid \
                  JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace \
                  JOIN pg_catalog.pg_constraint con ON con.oid=t.tgconstraint \
                  JOIN pg_catalog.pg_namespace cn ON cn.oid=con.connamespace \
                 WHERE n.nspname IN ('control','memory','readmodel') \
                   AND cn.nspname='foreman_execution' AND t.tgisinternal \
                   AND t.tgenabled='O' AND t.tgconstraint<>0), \
                (SELECT pg_catalog.count(*) FROM pg_catalog.pg_type t \
                  JOIN pg_catalog.pg_namespace n ON n.oid=t.typnamespace \
                 WHERE n.nspname='foreman_execution' \
                   AND (NOT t.typisdefined OR t.typtype IN ('d','e','p','r','m'))), \
                (SELECT pg_catalog.count(*) FROM pg_catalog.pg_type t \
                  JOIN pg_catalog.pg_namespace n ON n.oid=t.typnamespace \
                 WHERE n.nspname='foreman_execution'), \
                ((SELECT pg_catalog.count(*) FROM pg_catalog.pg_rewrite w \
                   JOIN pg_catalog.pg_class c ON c.oid=w.ev_class \
                   JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace \
                  WHERE n.nspname='foreman_execution' AND w.rulename<>'_RETURN') + \
                 (SELECT pg_catalog.count(*) FROM pg_catalog.pg_policy p \
                   JOIN pg_catalog.pg_class c ON c.oid=p.polrelid \
                   JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace \
                  WHERE n.nspname='foreman_execution') + \
                 (SELECT pg_catalog.count(*) FROM pg_catalog.pg_inherits i \
                   JOIN pg_catalog.pg_class child ON child.oid=i.inhrelid \
                   JOIN pg_catalog.pg_namespace child_ns ON child_ns.oid=child.relnamespace \
                   JOIN pg_catalog.pg_class parent ON parent.oid=i.inhparent \
                   JOIN pg_catalog.pg_namespace parent_ns ON parent_ns.oid=parent.relnamespace \
                  WHERE child_ns.nspname='foreman_execution' \
                     OR parent_ns.nspname='foreman_execution'))",
            &[],
        )
        .map_err(|error| map_postgres_error(&error, PostgresStoreSetupErrorKind::CorruptCatalog))?;
    for (source_index, expected) in [
        (0, 17_i64),
        (1, 41),
        (2, 43),
        (3, 17),
        (4, 43),
        (5, 0),
        (6, MANAGED_FOREMAN_RUNTIME_FUNCTION_COUNT),
        (10, 0),
        (11, 92),
        (12, 92),
        (13, MANAGED_FOREMAN_CONTROL_INTERNAL_TRIGGER_COUNT),
        (14, 0),
        (15, 34),
        (16, 0),
    ] {
        let observed = row_value::<i64>(
            &shape,
            source_index,
            PostgresStoreSetupErrorKind::CorruptCatalog,
        )?;
        if observed != expected {
            return Err(catalog_error());
        }
    }
    for (index, expected) in [(7, false), (8, true), (9, false)] {
        let observed =
            row_value::<bool>(&shape, index, PostgresStoreSetupErrorKind::PermissionDenied)?;
        if observed != expected {
            return Err(permission_error());
        }
    }

    let unmodeled = client
        .query_one(
            "SELECT \
             ((SELECT pg_catalog.count(*) FROM pg_catalog.pg_collation x \
                JOIN pg_catalog.pg_namespace n ON n.oid=x.collnamespace WHERE n.nspname='foreman_execution') + \
              (SELECT pg_catalog.count(*) FROM pg_catalog.pg_conversion x \
                JOIN pg_catalog.pg_namespace n ON n.oid=x.connamespace WHERE n.nspname='foreman_execution') + \
              (SELECT pg_catalog.count(*) FROM pg_catalog.pg_operator x \
                JOIN pg_catalog.pg_namespace n ON n.oid=x.oprnamespace WHERE n.nspname='foreman_execution') + \
              (SELECT pg_catalog.count(*) FROM pg_catalog.pg_opclass x \
                JOIN pg_catalog.pg_namespace n ON n.oid=x.opcnamespace WHERE n.nspname='foreman_execution') + \
              (SELECT pg_catalog.count(*) FROM pg_catalog.pg_opfamily x \
                JOIN pg_catalog.pg_namespace n ON n.oid=x.opfnamespace WHERE n.nspname='foreman_execution') + \
              (SELECT pg_catalog.count(*) FROM pg_catalog.pg_statistic_ext x \
                JOIN pg_catalog.pg_namespace n ON n.oid=x.stxnamespace WHERE n.nspname='foreman_execution') + \
              (SELECT pg_catalog.count(*) FROM pg_catalog.pg_ts_config x \
                JOIN pg_catalog.pg_namespace n ON n.oid=x.cfgnamespace WHERE n.nspname='foreman_execution') + \
              (SELECT pg_catalog.count(*) FROM pg_catalog.pg_ts_dict x \
                JOIN pg_catalog.pg_namespace n ON n.oid=x.dictnamespace WHERE n.nspname='foreman_execution') + \
              (SELECT pg_catalog.count(*) FROM pg_catalog.pg_ts_parser x \
                JOIN pg_catalog.pg_namespace n ON n.oid=x.prsnamespace WHERE n.nspname='foreman_execution') + \
              (SELECT pg_catalog.count(*) FROM pg_catalog.pg_ts_template x \
                JOIN pg_catalog.pg_namespace n ON n.oid=x.tmplnamespace WHERE n.nspname='foreman_execution')), \
             (SELECT pg_catalog.count(*) FROM pg_catalog.pg_default_acl d \
               JOIN pg_catalog.pg_namespace n ON n.oid=d.defaclnamespace \
              WHERE n.nspname='foreman_execution'), \
             (SELECT pg_catalog.count(*) FROM ( \
                SELECT t.oid \
                  FROM pg_catalog.pg_type t \
                  JOIN pg_catalog.pg_namespace n ON n.oid=t.typnamespace \
                  LEFT JOIN LATERAL pg_catalog.aclexplode( \
                    COALESCE(t.typacl,pg_catalog.acldefault('T',t.typowner))) acl ON TRUE \
                 WHERE n.nspname='foreman_execution' \
                 GROUP BY t.oid,t.typowner \
                HAVING pg_catalog.count(acl.privilege_type)<>2 \
                    OR pg_catalog.count(*) FILTER (WHERE acl.grantee=0 \
                        AND acl.grantor=t.typowner \
                        AND acl.privilege_type='USAGE' AND NOT acl.is_grantable)<>1 \
                    OR pg_catalog.count(*) FILTER (WHERE acl.grantee=t.typowner \
                        AND acl.grantor=t.typowner \
                        AND acl.privilege_type='USAGE' AND NOT acl.is_grantable)<>1 \
             ) type_acl_drift), \
             (SELECT pg_catalog.count(*) FROM pg_catalog.pg_cast c \
              WHERE c.castsource IN (SELECT t.oid FROM pg_catalog.pg_type t \
                    JOIN pg_catalog.pg_namespace n ON n.oid=t.typnamespace \
                   WHERE n.nspname='foreman_execution') \
                 OR c.casttarget IN (SELECT t.oid FROM pg_catalog.pg_type t \
                    JOIN pg_catalog.pg_namespace n ON n.oid=t.typnamespace \
                   WHERE n.nspname='foreman_execution') \
                  OR c.castfunc IN (SELECT p.oid FROM pg_catalog.pg_proc p \
                     JOIN pg_catalog.pg_namespace n ON n.oid=p.pronamespace \
                    WHERE n.nspname='foreman_execution')), \
             (SELECT pg_catalog.count(*) FROM pg_catalog.pg_transform tr \
              WHERE tr.trftype IN (SELECT t.oid FROM pg_catalog.pg_type t \
                    JOIN pg_catalog.pg_namespace n ON n.oid=t.typnamespace \
                   WHERE n.nspname='foreman_execution') \
                 OR tr.trffromsql IN (SELECT p.oid FROM pg_catalog.pg_proc p \
                    JOIN pg_catalog.pg_namespace n ON n.oid=p.pronamespace \
                   WHERE n.nspname='foreman_execution') \
                 OR tr.trftosql IN (SELECT p.oid FROM pg_catalog.pg_proc p \
                    JOIN pg_catalog.pg_namespace n ON n.oid=p.pronamespace \
                   WHERE n.nspname='foreman_execution'))",
            &[],
        )
        .map_err(|error| map_postgres_error(&error, PostgresStoreSetupErrorKind::CorruptCatalog))?;
    for index in 0..5 {
        let observed = row_value::<i64>(
            &unmodeled,
            index,
            PostgresStoreSetupErrorKind::CorruptCatalog,
        )?;
        if observed != 0 {
            return Err(catalog_error());
        }
    }

    let relation_oids = client
        .query(
            "SELECT c.oid::bigint FROM pg_catalog.pg_class c \
              JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace \
             WHERE n.nspname='foreman_execution' ORDER BY c.oid",
            &[],
        )
        .map_err(|error| map_postgres_error(&error, PostgresStoreSetupErrorKind::CorruptCatalog))?
        .iter()
        .map(|row| row_value::<i64>(row, 0, PostgresStoreSetupErrorKind::CorruptCatalog))
        .collect::<Result<Vec<_>, _>>()?;
    let function_oids = client
        .query(
            "SELECT p.oid::bigint FROM pg_catalog.pg_proc p \
              JOIN pg_catalog.pg_namespace n ON n.oid=p.pronamespace \
             WHERE n.nspname='foreman_execution' ORDER BY p.oid",
            &[],
        )
        .map_err(|error| map_postgres_error(&error, PostgresStoreSetupErrorKind::CorruptCatalog))?
        .iter()
        .map(|row| row_value::<i64>(row, 0, PostgresStoreSetupErrorKind::CorruptCatalog))
        .collect::<Result<Vec<_>, _>>()?;
    if relation_oids.len() != 58 || function_oids.len() != 43 {
        return Err(catalog_error());
    }
    Ok(Some(ManagedForemanPrincipalProfile {
        relation_oids,
        function_oids,
        binding,
    }))
}

#[allow(clippy::too_many_lines)]
fn verify_runtime_submission_schema_v7<C: GenericClient>(
    client: &mut C,
    target: &MigrationTarget,
    manifest: &ManifestEvidence,
    runtime_active: bool,
) -> Result<String, PostgresStoreSetupError> {
    let rows = read_history_rows(client)?;
    let retained = retained_history_rows(&rows)?;
    let compatibility = read_retained_schema_compatibility(client)?;
    match classify_retained_history_rows(&retained, &compatibility) {
        RetainedHistoryClassification::ExactSupported
            if compatibility.manifest_sha256 == manifest.manifest_sha256().as_str() => {}
        RetainedHistoryClassification::StrictFutureSuffix => {
            return Err(PostgresStoreSetupError::new(
                PostgresStoreSetupErrorKind::UnsupportedFutureSchema,
            ));
        }
        RetainedHistoryClassification::ExactSupported | RetainedHistoryClassification::Corrupt => {
            return Err(history_error());
        }
    }
    verify_owned_catalog_signature_profile(client, &SCHEMA_V7_OWNED_CATALOG_SIGNATURES)?;
    verify_schema_header_comments(client, "V7")?;
    let managed_foreman = verify_optional_managed_foreman_extension(client, target)?;
    if managed_foreman
        .as_ref()
        .is_some_and(|profile| profile.binding != ManagedForemanStoreBinding::StoreV7Base)
    {
        return Err(catalog_error());
    }
    let mut expected_forbidden_objects = SCHEMA_V7_FORBIDDEN_SCHEMA_OBJECT_COUNTS;
    if managed_foreman.is_some() {
        expected_forbidden_objects[7] += MANAGED_FOREMAN_CONTROL_INTERNAL_TRIGGER_COUNT;
    }
    verify_schema_v6_v7_forbidden_object_profile(client, &expected_forbidden_objects)?;
    verify_exact_default_acl_signature(client)?;
    verify_autonomy_receipt_profile(client)?;
    verify_forbidden_namespace_objects(client)?;
    verify_effective_default_privileges(client)?;
    verify_v7_ingress_ambiguity_profile(client)?;

    let catalog = client
        .query_one(
            "SELECT \
                (SELECT count(*)::bigint FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace \
                  WHERE n.nspname='control' AND c.relkind='r'), \
                (SELECT count(*)::bigint FROM pg_proc p JOIN pg_namespace n ON n.oid=p.pronamespace \
                  WHERE n.nspname='control'), \
                (SELECT count(*) FILTER (WHERE pg_catalog.has_function_privilege(\
                    'lattice_runtime',p.oid,'EXECUTE'))::bigint \
                   FROM pg_proc p JOIN pg_namespace n ON n.oid=p.pronamespace \
                  WHERE n.nspname='control'), \
                (SELECT count(*)::bigint FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace \
                  WHERE n.nspname='control' AND c.relname IN (
                    'task_submission_envelopes','task_ingress_claims',
                    'task_ingress_historical_ambiguities') \
                    AND c.relkind='r' AND pg_get_userbyid(c.relowner)='lattice_migrator'), \
                COALESCE(pg_catalog.has_table_privilege('lattice_runtime',\
                    'control.task_submission_envelopes','SELECT,INSERT,UPDATE,DELETE,TRUNCATE,REFERENCES,TRIGGER'),false) \
                  OR COALESCE(pg_catalog.has_table_privilege('lattice_runtime',\
                    'control.task_ingress_claims','SELECT,INSERT,UPDATE,DELETE,TRUNCATE,REFERENCES,TRIGGER'),false) \
                  OR COALESCE(pg_catalog.has_table_privilege('lattice_runtime',\
                    'control.task_ingress_historical_ambiguities','SELECT,INSERT,UPDATE,DELETE,TRUNCATE,REFERENCES,TRIGGER'),false), \
                (SELECT count(*)::bigint FROM pg_proc p JOIN pg_namespace n ON n.oid=p.pronamespace \
                  WHERE n.nspname='control' AND p.proname IN (\
                    'task_submission_prepare_v1','task_submission_record_v1',\
                    'task_submission_read_by_task_ref_v1','task_submission_read_by_request_v1',\
                    'task_ingress_prepare_v1','task_ingress_record_v1',\
                    'task_ingress_read_by_request_v1',\
                    'task_ingress_historical_closure_v1') \
                    AND pg_catalog.has_function_privilege('lattice_runtime',p.oid,'EXECUTE')), \
                (SELECT count(*)::bigint FROM pg_attribute a \
                  JOIN pg_class c ON c.oid=a.attrelid \
                  JOIN pg_namespace n ON n.oid=c.relnamespace \
                 WHERE n.nspname='control' AND c.relname='task_ledger_streams' \
                   AND a.attnum>0 AND NOT a.attisdropped AND (\
                     (a.attname='task_subject_kind' AND a.atttypid='varchar'::regtype AND a.attnotnull) OR \
                     (a.attname='task_subject_digest' AND a.atttypid='bytea'::regtype AND a.attnotnull) OR \
                     (a.attname='task_spec_digest' AND a.atttypid='bytea'::regtype AND NOT a.attnotnull) OR \
                     (a.attname='accounting_currency' AND a.atttypid='bpchar'::regtype \
                      AND a.atttypmod=7 AND NOT a.attnotnull)\
                   )), \
                (SELECT count(*)::bigint FROM pg_proc p JOIN pg_namespace n ON n.oid=p.pronamespace \
                  WHERE n.nspname='control' AND p.proname IN (\
                    'task_ledger_read_head_v4','task_ledger_finalize_general_intake_v1') \
                    AND pg_catalog.has_function_privilege('lattice_runtime',p.oid,'EXECUTE')), \
                (SELECT count(*)::bigint FROM pg_proc p JOIN pg_namespace n ON n.oid=p.pronamespace \
                  WHERE n.nspname='control' AND p.proname='task_ledger_read_head_v3' \
                    AND pg_catalog.has_function_privilege('lattice_runtime',p.oid,'EXECUTE')), \
                (SELECT count(*)::bigint FROM pg_constraint k \
                  JOIN pg_class c ON c.oid=k.conrelid \
                  JOIN pg_namespace n ON n.oid=c.relnamespace \
                 WHERE n.nspname='control' AND c.relname='task_ledger_streams' \
                   AND k.conname='task_ledger_streams_subject_shape' \
                   AND k.contype='c' AND k.convalidated)",
            &[],
        )
        .map_err(|error| map_postgres_error(&error, PostgresStoreSetupErrorKind::CorruptCatalog))?;
    let table_count = row_value::<i64>(&catalog, 0, PostgresStoreSetupErrorKind::CorruptCatalog)?;
    let retained_functions =
        row_value::<i64>(&catalog, 1, PostgresStoreSetupErrorKind::CorruptCatalog)?;
    let runtime_functions =
        row_value::<i64>(&catalog, 2, PostgresStoreSetupErrorKind::PermissionDenied)?;
    let submission_table =
        row_value::<i64>(&catalog, 3, PostgresStoreSetupErrorKind::CorruptCatalog)?;
    let direct_table =
        row_value::<bool>(&catalog, 4, PostgresStoreSetupErrorKind::PermissionDenied)?;
    let submission_runtime_functions =
        row_value::<i64>(&catalog, 5, PostgresStoreSetupErrorKind::PermissionDenied)?;
    let subject_columns =
        row_value::<i64>(&catalog, 6, PostgresStoreSetupErrorKind::CorruptCatalog)?;
    let general_runtime_functions =
        row_value::<i64>(&catalog, 7, PostgresStoreSetupErrorKind::PermissionDenied)?;
    let legacy_head_runtime_functions =
        row_value::<i64>(&catalog, 8, PostgresStoreSetupErrorKind::PermissionDenied)?;
    let subject_constraint =
        row_value::<i64>(&catalog, 9, PostgresStoreSetupErrorKind::CorruptCatalog)?;
    if (
        table_count,
        retained_functions,
        runtime_functions,
        submission_table,
        direct_table,
        submission_runtime_functions,
        subject_columns,
        general_runtime_functions,
        legacy_head_runtime_functions,
        subject_constraint,
    ) != (20, 59, 30, 3, false, 8, 4, 2, 0, 1)
    {
        return Err(catalog_error());
    }

    let writer = client
        .query_one(
            "SELECT \
                (SELECT count(*)::bigint FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace \
                  WHERE n.nspname='writer_lease' AND c.relkind='r'), \
                (SELECT count(*)::bigint FROM pg_proc p JOIN pg_namespace n ON n.oid=p.pronamespace \
                  WHERE n.nspname='writer_lease'), \
                (SELECT count(*) FILTER (WHERE pg_catalog.has_function_privilege(\
                    'lattice_runtime',p.oid,'EXECUTE'))::bigint \
                   FROM pg_proc p JOIN pg_namespace n ON n.oid=p.pronamespace \
                  WHERE n.nspname='writer_lease'), \
                COALESCE(pg_catalog.has_schema_privilege('lattice_runtime','writer_lease','USAGE'),false)",
            &[],
        )
        .map_err(|error| map_postgres_error(&error, PostgresStoreSetupErrorKind::CorruptCatalog))?;
    let writer_tables = row_value::<i64>(&writer, 0, PostgresStoreSetupErrorKind::CorruptCatalog)?;
    let writer_functions =
        row_value::<i64>(&writer, 1, PostgresStoreSetupErrorKind::CorruptCatalog)?;
    let writer_runtime_functions =
        row_value::<i64>(&writer, 2, PostgresStoreSetupErrorKind::PermissionDenied)?;
    let writer_usage =
        row_value::<bool>(&writer, 3, PostgresStoreSetupErrorKind::PermissionDenied)?;
    if writer_tables != 5 || writer_runtime_functions != 7 || !writer_usage {
        return Err(catalog_error());
    }
    // Runtime has no table privilege in the Writer namespace by design. The
    // exact function count selects only the frozen v4 predecessor or its v5
    // append-only successor; the selected verifier then pins every function
    // body, comment, ACL, owner, argument list, and embedded SQL digest.
    let writer_v5 = match writer_functions {
        15 => {
            verify_writer_lease_exact_catalog_profile(
                client,
                &WRITER_LEASE_V4_CURRENT_CATALOG_SIGNATURES,
            )?;
            verify_writer_lease_v4_functions(client, true)?;
            verify_writer_lease_acl_closure(client, 8, true)?;
            false
        }
        17 => {
            let runtime_profile = classify_writer_lease_v5_runtime_profile(client)?;
            let catalog_profile = match runtime_profile {
                WriterLeaseV5RuntimeProfile::StoreV7Base => {
                    &WRITER_LEASE_V5_CURRENT_CATALOG_SIGNATURES
                }
                WriterLeaseV5RuntimeProfile::StoreV8Successor => {
                    &WRITER_LEASE_V5_STORE_V8_CURRENT_CATALOG_SIGNATURES
                }
            };
            verify_writer_lease_exact_catalog_profile(client, catalog_profile)?;
            verify_writer_lease_v5_functions(client, runtime_profile)?;
            verify_writer_lease_acl_closure(client, 10, true)?;
            true
        }
        _ => return Err(catalog_error()),
    };
    if managed_foreman.is_some() && !writer_v5 {
        return Err(catalog_error());
    }

    let migration = &migration_manifest()[6];
    let candidate = ForemanSchemaV6Candidate::from_migration_bytes(
        migration.ordinal(),
        migration.id(),
        migration.path(),
        migration.schema_version(),
        migration.reader_compatibility(),
        migration.writer_compatibility(),
        FOREMAN_COORDINATION_STREAM_IDENTITY,
        FOREMAN_COORDINATION_EVENT_IDENTITY,
        migration.bytes(),
    )
    .map_err(|_| catalog_error())?;
    let _verified_profile = verify_foreman_schema_v6_profile(
        &candidate,
        &ForemanSchemaV6CatalogAcl::exact_foreman_coordination(),
        WriterLeaseV3Profile::Current,
    )
    .map_err(|_| catalog_error())?;
    if runtime_active {
        verify_runtime_admission_present(client)?;
    } else {
        verify_stopped_admission(client)?;
    }
    let expected_dangerous_functions = SCHEMA_V7_WRITER_V4_DANGEROUS_FUNCTION_COUNT
        + if managed_foreman.is_some() {
            MANAGED_FOREMAN_RUNTIME_FUNCTION_COUNT
        } else {
            0
        };
    verify_exact_principal_database_boundary(
        client,
        expected_dangerous_functions,
        true,
        managed_foreman.as_ref(),
    )?;
    read_database_identity(client, target)
}

fn verify_runtime_external_adoption_schema_v8<C: GenericClient>(
    client: &mut C,
    target: &MigrationTarget,
    manifest: &ManifestEvidence,
    runtime_active: bool,
) -> Result<String, PostgresStoreSetupError> {
    let rows = read_history_rows(client)?;
    let expected = migration_manifest()
        .get(..manifest.entry_count())
        .ok_or_else(history_error)?;
    verify_history_rows(&rows, expected)?;
    let compatibility = read_retained_schema_compatibility(client)?;
    if compatibility.manifest_sha256 != manifest.manifest_sha256().as_str()
        || compatibility.versions != [8, 8, 8, 8, 8]
    {
        return Err(history_error());
    }
    verify_schema_header_comments(client, "V7")?;
    let managed_foreman = verify_optional_managed_foreman_extension(client, target)?;
    if runtime_active
        && managed_foreman
            .as_ref()
            .is_some_and(|profile| profile.binding != ManagedForemanStoreBinding::StoreV8Rebound)
    {
        return Err(catalog_error());
    }
    verify_schema_v8_forbidden_object_profile(client, managed_foreman.is_some())?;
    if manifest.entry_count() == migration_manifest().len() {
        verify_owned_catalog_signature_profile(client, &SCHEMA_V8_OWNED_CATALOG_SIGNATURES)?;
        verify_store_v8_runtime_successor_functions(client)?;
        verify_writer_lease_v5_store_v8_successor(client)?;
        verify_exact_default_acl_signature(client)?;
        verify_autonomy_receipt_profile(client)?;
        verify_forbidden_namespace_objects(client)?;
        verify_effective_default_privileges(client)?;
    }
    verify_v7_ingress_ambiguity_profile(client)?;
    let profile = client
        .query_one(
            "SELECT \
                (SELECT count(*)::bigint FROM pg_catalog.pg_class c \
                  JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace \
                  JOIN pg_catalog.pg_roles owner ON owner.oid=c.relowner \
                 WHERE n.nspname='control' AND c.relkind='r' \
                   AND c.relname IN ('external_verified_result_evidence', \
                                     'task_external_verified_result_adoptions') \
                   AND owner.rolname='lattice_migrator'), \
                (SELECT count(*)::bigint FROM pg_catalog.pg_class c \
                  JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace \
                 WHERE n.nspname='control' AND c.relkind='r' \
                   AND c.relname IN ('external_verified_result_evidence', \
                                     'task_external_verified_result_adoptions') \
                   AND pg_catalog.has_table_privilege('lattice_runtime',c.oid, \
                     'SELECT,INSERT,UPDATE,DELETE,TRUNCATE,REFERENCES,TRIGGER')), \
                (SELECT count(*)::bigint FROM pg_catalog.pg_proc p \
                  JOIN pg_catalog.pg_namespace n ON n.oid=p.pronamespace \
                 WHERE n.nspname='control' AND p.proname IN (\
                    'external_verified_result_evidence_read_v1',\
                    'external_verified_result_adoption_preflight_v1',\
                    'external_verified_result_adoption_bind_v1') \
                   AND p.prosecdef AND pg_catalog.has_function_privilege(\
                     'lattice_runtime',p.oid,'EXECUTE')), \
                (SELECT count(*)::bigint FROM pg_catalog.pg_constraint k \
                  JOIN pg_catalog.pg_class c ON c.oid=k.conrelid \
                  JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace \
                 WHERE n.nspname='control' AND c.relname='task_external_verified_result_adoptions' \
                   AND k.contype='f' AND k.convalidated), \
                (SELECT count(*)::bigint FROM pg_catalog.pg_constraint k \
                  JOIN pg_catalog.pg_class c ON c.oid=k.conrelid \
                  JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace \
                 WHERE n.nspname='control' AND c.relname='task_ledger_events' \
                   AND k.conname='task_ledger_events_closed_values' \
                   AND pg_catalog.pg_get_constraintdef(k.oid,false) LIKE '%EXTERNAL_VERIFIED_RESULT_ADOPTED%')",
            &[],
        )
        .map_err(|error| map_postgres_error(&error, PostgresStoreSetupErrorKind::CorruptCatalog))?;
    for (index, expected) in [(0, 2_i64), (1, 0), (2, 3), (3, 3), (4, 1)] {
        if row_value::<i64>(&profile, index, PostgresStoreSetupErrorKind::CorruptCatalog)?
            != expected
        {
            return Err(catalog_error());
        }
    }
    if runtime_active {
        verify_runtime_admission_present(client)?;
    } else {
        verify_stopped_admission(client)?;
    }
    let expected_dangerous_functions = SCHEMA_V8_WRITER_V5_DANGEROUS_FUNCTION_COUNT
        + if managed_foreman.is_some() {
            MANAGED_FOREMAN_RUNTIME_FUNCTION_COUNT
        } else {
            0
        };
    verify_exact_principal_database_boundary(
        client,
        expected_dangerous_functions,
        true,
        managed_foreman.as_ref(),
    )?;
    read_database_identity(client, target)
}

#[allow(clippy::too_many_lines)]
fn verify_store_v8_runtime_successor_functions<C: GenericClient>(
    client: &mut C,
) -> Result<(), PostgresStoreSetupError> {
    let successor = migration_manifest().get(9).ok_or_else(history_error)?;
    let sql = std::str::from_utf8(successor.bytes()).map_err(|_| history_error())?;
    let rows = client
        .query(
            "SELECT p.proname::text,p.prokind::text,l.lanname,r.rolname,p.prosecdef, \
                    p.provolatile::text,p.proparallel::text, \
                    COALESCE(pg_catalog.array_to_string(p.proconfig,','),'<NULL>'), \
                    pg_catalog.has_function_privilege('lattice_runtime',p.oid,'EXECUTE'), \
                    p.prosrc::text \
               FROM pg_catalog.pg_proc p \
               JOIN pg_catalog.pg_namespace n ON n.oid=p.pronamespace \
               JOIN pg_catalog.pg_language l ON l.oid=p.prolang \
               JOIN pg_catalog.pg_roles r ON r.oid=p.proowner \
              WHERE n.nspname='control' AND p.proname IN ( \
                    'store_prepare_v5','store_finalize_v5','store_current_head_v5', \
                    'task_ledger_prepare_v3','task_ledger_read_head_v3', \
                    'task_ledger_read_events_v3','task_ledger_read_commands_v3', \
                    'task_ledger_finalize_v3','task_ledger_finalize_general_intake_v1', \
                    'project_registry_prepare_v2','project_registry_read_state_v2', \
                    'project_registry_read_observations_v2','project_registry_read_projects_v2', \
                    'project_registry_read_commands_v2','project_registry_read_reservations_v2', \
                    'project_registry_stage_command_v2','project_registry_stage_project_v2', \
                    'project_registry_finalize_v2') \
              ORDER BY p.proname,pg_catalog.pg_get_function_identity_arguments(p.oid)",
            &[],
        )
        .map_err(|error| map_postgres_error(&error, PostgresStoreSetupErrorKind::CorruptCatalog))?;
    let descriptors = [
        (
            "project_registry_finalize_v2",
            "lattice_project_registry_finalize_v2",
            "v",
            true,
        ),
        (
            "project_registry_prepare_v2",
            "lattice_project_registry_prepare_v2",
            "v",
            true,
        ),
        (
            "project_registry_read_commands_v2",
            "lattice_project_registry_read_commands_v2",
            "s",
            true,
        ),
        (
            "project_registry_read_observations_v2",
            "lattice_project_registry_read_observations_v2",
            "s",
            true,
        ),
        (
            "project_registry_read_projects_v2",
            "lattice_project_registry_read_projects_v2",
            "s",
            true,
        ),
        (
            "project_registry_read_reservations_v2",
            "lattice_project_registry_read_reservations_v2",
            "s",
            true,
        ),
        (
            "project_registry_read_state_v2",
            "lattice_project_registry_read_state_v2",
            "s",
            true,
        ),
        (
            "project_registry_stage_command_v2",
            "lattice_project_registry_stage_command_v2",
            "v",
            true,
        ),
        (
            "project_registry_stage_project_v2",
            "lattice_project_registry_stage_project_v2",
            "v",
            true,
        ),
        (
            "store_current_head_v5",
            "lattice_store_current_head_v5",
            "s",
            true,
        ),
        ("store_finalize_v5", "lattice_store_finalize_v5", "v", true),
        ("store_prepare_v5", "lattice_store_prepare_v5", "v", true),
        (
            "task_ledger_finalize_general_intake_v1",
            "lattice_task_ledger_finalize_general_intake_v1",
            "v",
            true,
        ),
        (
            "task_ledger_finalize_v3",
            "lattice_task_ledger_finalize_v3",
            "v",
            true,
        ),
        (
            "task_ledger_prepare_v3",
            "lattice_task_ledger_prepare_v3",
            "v",
            true,
        ),
        (
            "task_ledger_read_commands_v3",
            "lattice_task_ledger_read_commands_v3",
            "s",
            true,
        ),
        (
            "task_ledger_read_events_v3",
            "lattice_task_ledger_read_events_v3",
            "s",
            true,
        ),
        (
            "task_ledger_read_head_v3",
            "lattice_task_ledger_read_head_v3",
            "s",
            false,
        ),
    ];
    if rows.len() != descriptors.len() {
        return Err(catalog_error());
    }
    for (row, (name, delimiter, volatility, runtime_execute)) in rows.iter().zip(descriptors) {
        if row_value::<String>(row, 0, PostgresStoreSetupErrorKind::CorruptCatalog)? != name
            || row_value::<String>(row, 1, PostgresStoreSetupErrorKind::CorruptCatalog)? != "f"
            || row_value::<String>(row, 2, PostgresStoreSetupErrorKind::CorruptCatalog)?
                != "plpgsql"
            || row_value::<String>(row, 3, PostgresStoreSetupErrorKind::CorruptCatalog)?
                != "lattice_migrator"
            || !row_value::<bool>(row, 4, PostgresStoreSetupErrorKind::CorruptCatalog)?
            || row_value::<String>(row, 5, PostgresStoreSetupErrorKind::CorruptCatalog)?
                != volatility
            || row_value::<String>(row, 6, PostgresStoreSetupErrorKind::CorruptCatalog)? != "u"
            || row_value::<String>(row, 7, PostgresStoreSetupErrorKind::CorruptCatalog)?
                != "search_path=pg_catalog,row_security=on,lock_timeout=5s,statement_timeout=30s"
            || row_value::<bool>(row, 8, PostgresStoreSetupErrorKind::PermissionDenied)?
                != runtime_execute
            || row_value::<String>(row, 9, PostgresStoreSetupErrorKind::CorruptCatalog)?
                != embedded_writer_function_source(sql, delimiter)?
        {
            return Err(catalog_error());
        }
    }
    Ok(())
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
                5 => {
                    verify_history_rows(&rows, &migration_manifest()[..5])?;
                    Ok(InstalledManifestState::ExactV4Prefix)
                }
                6 => {
                    verify_history_rows(&rows, &migration_manifest()[..6])?;
                    Ok(InstalledManifestState::ExactV5Prefix)
                }
                7 => {
                    verify_history_rows(&rows, &migration_manifest()[..7])?;
                    Ok(InstalledManifestState::ExactV6Prefix)
                }
                8 => {
                    verify_history_rows(&rows, &migration_manifest()[..8])?;
                    Ok(InstalledManifestState::ExactV7Prefix)
                }
                9 => {
                    verify_history_rows(&rows, &migration_manifest()[..9])?;
                    Ok(InstalledManifestState::ExactV8LegacyPrefix)
                }
                length if length == migration_manifest().len() => {
                    verify_history_rows(&rows, migration_manifest())?;
                    Ok(InstalledManifestState::ExactV8Full)
                }
                length if length > migration_manifest().len() => {
                    match classify_retained_history(client)? {
                        RetainedHistoryClassification::StrictFutureSuffix => {
                            Err(PostgresStoreSetupError::new(
                                PostgresStoreSetupErrorKind::UnsupportedFutureSchema,
                            ))
                        }
                        RetainedHistoryClassification::ExactSupported
                        | RetainedHistoryClassification::Corrupt => Err(history_error()),
                    }
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
    apply_entries_until(client, applied_prefix_len, migration_manifest().len())
}

fn apply_entries_until<C: GenericClient>(
    client: &mut C,
    applied_prefix_len: usize,
    target_prefix_len: usize,
) -> Result<usize, PostgresStoreSetupError> {
    if applied_prefix_len > target_prefix_len || target_prefix_len > migration_manifest().len() {
        return Err(history_error());
    }
    let mut executable_count = 0usize;
    for entry in &migration_manifest()[applied_prefix_len..target_prefix_len] {
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

    for entry in &migration_manifest()[applied_prefix_len..target_prefix_len] {
        insert_history(client, entry)?;
    }
    Ok(executable_count)
}

fn insert_current_compatibility<C: GenericClient>(
    client: &mut C,
    manifest: &ManifestEvidence,
    applied_prefix_len: usize,
) -> Result<(), PostgresStoreSetupError> {
    let current = migration_manifest()
        .get(applied_prefix_len.saturating_sub(1))
        .ok_or_else(|| {
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
                &5_i16,
                &5_i16,
                &5_i16,
                &5_i16,
                &5_i16,
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
             SET manifest_sha256 = $1, current_schema_version = 5, \
                 min_reader = 5, max_reader = 5, min_writer = 5, max_writer = 5, \
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
             SET manifest_sha256 = $1, current_schema_version = 5, \
                 min_reader = 5, max_reader = 5, min_writer = 5, max_writer = 5, \
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

fn advance_compatibility_from_v4<C: GenericClient>(
    client: &mut C,
    v4_manifest: &ManifestEvidence,
    current_manifest: &ManifestEvidence,
) -> Result<(), PostgresStoreSetupError> {
    let updated = client
        .execute(
            "UPDATE ONLY control.schema_compatibility \
             SET manifest_sha256 = $1, current_schema_version = 5, \
                 min_reader = 5, max_reader = 5, min_writer = 5, max_writer = 5, \
                 updated_at = clock_timestamp() \
             WHERE singleton = true \
               AND manifest_sha256 = $2 \
               AND current_schema_version = 4 \
               AND min_reader = 4 AND max_reader = 4 \
               AND min_writer = 4 AND max_writer = 4",
            &[
                &current_manifest.manifest_sha256().as_str(),
                &v4_manifest.manifest_sha256().as_str(),
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

fn advance_compatibility_from_v5<C: GenericClient>(
    client: &mut C,
    v5_manifest: &ManifestEvidence,
    v6_manifest: &ManifestEvidence,
) -> Result<(), PostgresStoreSetupError> {
    let updated = client
        .execute(
            "UPDATE ONLY control.schema_compatibility \
             SET manifest_sha256 = $1, current_schema_version = 6, \
                 min_reader = 6, max_reader = 6, min_writer = 6, max_writer = 6, \
                 updated_at = clock_timestamp() \
             WHERE singleton = true \
               AND manifest_sha256 = $2 \
               AND current_schema_version = 5 \
               AND min_reader = 5 AND max_reader = 5 \
               AND min_writer = 5 AND max_writer = 5",
            &[
                &v6_manifest.manifest_sha256().as_str(),
                &v5_manifest.manifest_sha256().as_str(),
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

fn advance_compatibility_from_v6<C: GenericClient>(
    client: &mut C,
    v6_manifest: &ManifestEvidence,
    v7_manifest: &ManifestEvidence,
) -> Result<(), PostgresStoreSetupError> {
    let updated = client
        .execute(
            "UPDATE ONLY control.schema_compatibility \
             SET manifest_sha256 = $1, current_schema_version = 7, \
                 min_reader = 7, max_reader = 7, min_writer = 7, max_writer = 7, \
                 updated_at = clock_timestamp() \
             WHERE singleton = true \
               AND manifest_sha256 = $2 \
               AND current_schema_version = 6 \
               AND min_reader = 6 AND max_reader = 6 \
               AND min_writer = 6 AND max_writer = 6",
            &[
                &v7_manifest.manifest_sha256().as_str(),
                &v6_manifest.manifest_sha256().as_str(),
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

fn advance_compatibility_from_v7<C: GenericClient>(
    client: &mut C,
    v7_manifest: &ManifestEvidence,
    v8_manifest: &ManifestEvidence,
) -> Result<(), PostgresStoreSetupError> {
    let updated = client
        .execute(
            "UPDATE ONLY control.schema_compatibility \
             SET manifest_sha256 = $1, current_schema_version = 8, \
                 min_reader = 8, max_reader = 8, min_writer = 8, max_writer = 8, \
                 updated_at = clock_timestamp() \
             WHERE singleton = true \
               AND manifest_sha256 = $2 \
               AND current_schema_version = 7 \
               AND min_reader = 7 AND max_reader = 7 \
               AND min_writer = 7 AND max_writer = 7",
            &[
                &v8_manifest.manifest_sha256().as_str(),
                &v7_manifest.manifest_sha256().as_str(),
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

fn advance_compatibility_from_v8_legacy<C: GenericClient>(
    client: &mut C,
    legacy_v8_manifest: &ManifestEvidence,
    current_v8_manifest: &ManifestEvidence,
) -> Result<(), PostgresStoreSetupError> {
    let updated = client
        .execute(
            "UPDATE ONLY control.schema_compatibility \
             SET manifest_sha256 = $1, updated_at = clock_timestamp() \
             WHERE singleton = true \
               AND manifest_sha256 = $2 \
               AND current_schema_version = 8 \
               AND min_reader = 8 AND max_reader = 8 \
               AND min_writer = 8 AND max_writer = 8",
            &[
                &current_v8_manifest.manifest_sha256().as_str(),
                &legacy_v8_manifest.manifest_sha256().as_str(),
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
    let current_profile = classify_current_catalog_profile(client, manifest.schema_version())?;
    if matches!(
        current_profile,
        CatalogProfile::V5CodebaseMemoryV2UpgradePending
            | CatalogProfile::V5CodebaseMemoryV2WriterLeaseV2BridgePending
            | CatalogProfile::V5CodebaseMemoryV3WriterLeaseV2BridgePending
    ) && role != DatabaseRole::Migrator
    {
        return Err(PostgresStoreSetupError::new(
            PostgresStoreSetupErrorKind::CompatibilityMismatch,
        ));
    }
    verify_schema_objects(client, current_profile)?;
    verify_autonomy_receipt_profile(client)?;
    let rows = read_history_rows(client)?;
    verify_history_rows(&rows, &migration_manifest()[..manifest.entry_count()])?;
    verify_compatibility(client, manifest, current_profile)?;
    let database_uuid = read_database_identity(client, target)?;
    match current_profile {
        CatalogProfile::V5CodebaseMemoryV2UpgradePending
        | CatalogProfile::V5CodebaseMemoryV2WriterLeaseV2BridgePending => {
            verify_codebase_memory_v2_identity(client, target)?;
        }
        CatalogProfile::V5CodebaseMemoryV3Current
        | CatalogProfile::V5CodebaseMemoryV3WriterLeaseV2BridgePending
        | CatalogProfile::V5CodebaseMemoryV3WriterLeaseV2Current => {
            verify_codebase_memory_v3_identity_for_role(client, target, manifest, role)?;
        }
        CatalogProfile::PreSchema
        | CatalogProfile::V1
        | CatalogProfile::V2
        | CatalogProfile::V3
        | CatalogProfile::V3CodebaseMemoryV2
        | CatalogProfile::V3CodebaseMemoryV2WriterLeaseV1
        | CatalogProfile::V3CodebaseMemoryV2WriterLeaseV2Bridge
        | CatalogProfile::V4
        | CatalogProfile::V5 => {}
    }
    verify_stopped_admission(client)?;
    verify_roles_and_grants(client, current_profile)?;

    Ok(PostgresSchemaEvidence {
        database_uuid,
        manifest_sha256: manifest.manifest_sha256().clone(),
        schema_version: manifest.schema_version(),
        server_version_num,
        role,
        bootstrap_admission: BootstrapAdmission::StoppedNoLeader,
    })
}

fn verify_autonomy_receipt_profile<C: GenericClient>(
    client: &mut C,
) -> Result<(), PostgresStoreSetupError> {
    verify_autonomy_receipt_catalog_signature(client)?;
    let table = client
        .query_one(
            "SELECT owner.rolname, c.relkind::text, c.relpersistence::text, \
                    c.relhassubclass, c.relispartition, \
                    (SELECT count(*) FROM pg_attribute a \
                      WHERE a.attrelid = c.oid AND a.attnum > 0), \
                    (SELECT count(*) FROM pg_constraint con \
                      WHERE con.conrelid = c.oid), \
                    (SELECT count(*) FROM pg_trigger tr \
                      WHERE tr.tgrelid = c.oid AND NOT tr.tgisinternal), \
                    (SELECT count(*) FROM pg_policy p WHERE p.polrelid = c.oid) \
               FROM pg_class c \
               JOIN pg_namespace n ON n.oid = c.relnamespace \
               JOIN pg_roles owner ON owner.oid = c.relowner \
              WHERE n.nspname = 'control' \
                AND c.relname = 'task_ledger_autonomy_receipts'",
            &[],
        )
        .map_err(|error| map_postgres_error(&error, PostgresStoreSetupErrorKind::CorruptCatalog))?;
    if row_value::<String>(&table, 0, PostgresStoreSetupErrorKind::CorruptCatalog)?
        != DatabaseRole::Migrator.as_str()
        || row_value::<String>(&table, 1, PostgresStoreSetupErrorKind::CorruptCatalog)? != "r"
        || row_value::<String>(&table, 2, PostgresStoreSetupErrorKind::CorruptCatalog)? != "p"
        || row_value::<bool>(&table, 3, PostgresStoreSetupErrorKind::CorruptCatalog)?
        || row_value::<bool>(&table, 4, PostgresStoreSetupErrorKind::CorruptCatalog)?
        || row_value::<i64>(&table, 5, PostgresStoreSetupErrorKind::CorruptCatalog)? != 28
        || row_value::<i64>(&table, 6, PostgresStoreSetupErrorKind::CorruptCatalog)? != 10
        || row_value::<i64>(&table, 7, PostgresStoreSetupErrorKind::CorruptCatalog)? != 0
        || row_value::<i64>(&table, 8, PostgresStoreSetupErrorKind::CorruptCatalog)? != 0
    {
        return Err(catalog_error());
    }
    let rows = client
        .query(
            "SELECT n.nspname || '.' || p.proname || '(' || \
                    replace(pg_catalog.oidvectortypes(p.proargtypes), ' ', '') || ')', \
                    pg_get_userbyid(p.proowner), p.prosecdef, p.proleakproof, \
                    COALESCE(array_to_string(p.proconfig, ','), '<NULL>'), \
                    has_function_privilege('public', p.oid, 'EXECUTE'), \
                    has_function_privilege('lattice_runtime', p.oid, 'EXECUTE') \
               FROM pg_proc p JOIN pg_namespace n ON n.oid = p.pronamespace \
              WHERE n.nspname = 'control' AND p.proname IN ( \
                    'task_ledger_record_autonomy_receipt_v1', \
                    'task_ledger_read_autonomy_receipts_v1') \
              ORDER BY p.proname",
            &[],
        )
        .map_err(|error| map_postgres_error(&error, PostgresStoreSetupErrorKind::CorruptCatalog))?;
    let expected = BTreeSet::from([
        TASK_LEDGER_RECORD_AUTONOMY_RECEIPT_V1_IDENTITY.to_owned(),
        TASK_LEDGER_READ_AUTONOMY_RECEIPTS_V1_IDENTITY.to_owned(),
    ]);
    let mut actual = BTreeSet::new();
    for row in &rows {
        actual.insert(row_value::<String>(
            row,
            0,
            PostgresStoreSetupErrorKind::CorruptCatalog,
        )?);
        if row_value::<String>(row, 1, PostgresStoreSetupErrorKind::CorruptCatalog)?
            != DatabaseRole::Migrator.as_str()
            || !row_value::<bool>(row, 2, PostgresStoreSetupErrorKind::CorruptCatalog)?
            || row_value::<bool>(row, 3, PostgresStoreSetupErrorKind::CorruptCatalog)?
            || row_value::<String>(row, 4, PostgresStoreSetupErrorKind::CorruptCatalog)?
                != "search_path=pg_catalog,row_security=on,lock_timeout=5s,statement_timeout=30s"
            || row_value::<bool>(row, 5, PostgresStoreSetupErrorKind::PermissionDenied)?
            || !row_value::<bool>(row, 6, PostgresStoreSetupErrorKind::PermissionDenied)?
        {
            return Err(catalog_error());
        }
    }
    if actual != expected {
        return Err(catalog_error());
    }
    for role in [
        DatabaseRole::Runtime,
        DatabaseRole::Guardian,
        DatabaseRole::ReadOnly,
    ] {
        let privileges = client
            .query_one(
                "SELECT has_table_privilege($1, \
                    'control.task_ledger_autonomy_receipts', \
                    'SELECT,INSERT,UPDATE,DELETE,TRUNCATE,REFERENCES,TRIGGER,MAINTAIN')",
                &[&role.as_str()],
            )
            .map_err(|error| {
                map_postgres_error(&error, PostgresStoreSetupErrorKind::PermissionDenied)
            })?;
        if row_value::<bool>(
            &privileges,
            0,
            PostgresStoreSetupErrorKind::PermissionDenied,
        )? {
            return Err(permission_error());
        }
    }
    Ok(())
}

fn verify_autonomy_receipt_catalog_signature<C: GenericClient>(
    client: &mut C,
) -> Result<(), PostgresStoreSetupError> {
    let signature = catalog_signature(
        client,
        AUTONOMY_PROFILE_SIGNATURE_SQL,
        PostgresStoreSetupErrorKind::CorruptCatalog,
    )?;
    if signature != AUTONOMY_PROFILE_SIGNATURE {
        return Err(catalog_error());
    }
    Ok(())
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
    let mut profile = classify_current_catalog_profile(client, 3)?;
    if v3_upgrade_source_has_memory(profile)? {
        client
            .batch_execute(
                "LOCK TABLE memory.codebase_memory_analyses IN ACCESS EXCLUSIVE MODE; \
                 LOCK TABLE memory.codebase_memory_extension_identity IN ACCESS EXCLUSIVE MODE; \
                 LOCK TABLE memory.codebase_memory_extension_ledger IN ACCESS EXCLUSIVE MODE; \
                 LOCK TABLE memory.codebase_memory_receipts IN ACCESS EXCLUSIVE MODE; \
                 LOCK TABLE memory.codebase_memory_records IN ACCESS EXCLUSIVE MODE; \
                 LOCK TABLE memory.codebase_memory_reflections IN ACCESS EXCLUSIVE MODE; \
                 LOCK TABLE memory.codebase_memory_retrieval_audits IN ACCESS EXCLUSIVE MODE; \
                 LOCK TABLE memory.openclaw_gateway_commands IN ACCESS EXCLUSIVE MODE",
            )
            .map_err(|error| {
                map_postgres_error(&error, PostgresStoreSetupErrorKind::TransactionFailed)
            })?;
        let expected_profile = profile;
        profile = classify_current_catalog_profile(client, 3)?;
        if profile != expected_profile {
            return Err(PostgresStoreSetupError::new(
                PostgresStoreSetupErrorKind::CompatibilityMismatch,
            ));
        }
    }
    verify_schema_objects_with_contract(client, profile, true)?;
    let rows = read_history_rows(client)?;
    verify_history_rows(&rows, &migration_manifest()[..4])?;
    verify_compatibility(client, v3_manifest, profile)?;
    read_database_identity(client, target)?;
    if profile == CatalogProfile::V3CodebaseMemoryV2WriterLeaseV2Bridge {
        verify_codebase_memory_v2_identity(client, target)?;
    }
    verify_stopped_admission(client)?;
    verify_roles_and_grants_with_contract(client, profile, true)
}

fn v3_upgrade_source_has_memory(profile: CatalogProfile) -> Result<bool, PostgresStoreSetupError> {
    match profile {
        CatalogProfile::V3 => Ok(false),
        CatalogProfile::V3CodebaseMemoryV2
        | CatalogProfile::V3CodebaseMemoryV2WriterLeaseV2Bridge => Ok(true),
        CatalogProfile::PreSchema
        | CatalogProfile::V1
        | CatalogProfile::V2
        | CatalogProfile::V3CodebaseMemoryV2WriterLeaseV1
        | CatalogProfile::V4
        | CatalogProfile::V5
        | CatalogProfile::V5CodebaseMemoryV2UpgradePending
        | CatalogProfile::V5CodebaseMemoryV3Current
        | CatalogProfile::V5CodebaseMemoryV2WriterLeaseV2BridgePending
        | CatalogProfile::V5CodebaseMemoryV3WriterLeaseV2BridgePending
        | CatalogProfile::V5CodebaseMemoryV3WriterLeaseV2Current => Err(
            PostgresStoreSetupError::new(PostgresStoreSetupErrorKind::CompatibilityMismatch),
        ),
    }
}

#[allow(clippy::too_many_lines)]
#[derive(Clone, Copy)]
struct CodebaseMemoryIdentityProfile<'a> {
    extension_id: &'a str,
    extension_schema_version: i16,
    extension_path: &'a str,
    extension_sql_sha256: &'a str,
    extension_manifest_sha256: &'a str,
    database_uuid: &'a str,
    database_identity_sha256: &'a str,
    global_schema_version: i16,
    global_manifest_sha256: &'a str,
}

fn codebase_memory_identity_profile_matches(
    actual: CodebaseMemoryIdentityProfile<'_>,
    expected: CodebaseMemoryIdentityProfile<'_>,
) -> bool {
    actual.extension_id == expected.extension_id
        && actual.extension_schema_version == expected.extension_schema_version
        && actual.extension_path == expected.extension_path
        && actual.extension_sql_sha256 == expected.extension_sql_sha256
        && actual.extension_manifest_sha256 == expected.extension_manifest_sha256
        && actual.database_uuid == expected.database_uuid
        && actual.database_identity_sha256 == expected.database_identity_sha256
        && actual.global_schema_version == expected.global_schema_version
        && actual.global_manifest_sha256 == expected.global_manifest_sha256
}

// Keep the singleton and ledger-row comparisons together so a historical
// profile cannot pass after only one side of its frozen identity is checked.
#[allow(clippy::too_many_lines)]
fn verify_codebase_memory_v2_identity<C: GenericClient>(
    client: &mut C,
    target: &MigrationTarget,
) -> Result<(), PostgresStoreSetupError> {
    let identities = client
        .query(
            "SELECT extension_id::text, extension_schema_version, extension_path::text, \
                    btrim(extension_sql_sha256)::text, \
                    btrim(extension_manifest_sha256)::text, database_uuid::text, \
                    btrim(database_identity_sha256)::text, global_schema_version, \
                    btrim(global_manifest_sha256)::text \
               FROM ONLY memory.codebase_memory_extension_identity \
              WHERE singleton",
            &[],
        )
        .map_err(|error| {
            map_postgres_error(&error, PostgresStoreSetupErrorKind::CompatibilityMismatch)
        })?;
    if identities.len() != 1 {
        return Err(PostgresStoreSetupError::new(
            PostgresStoreSetupErrorKind::CompatibilityMismatch,
        ));
    }
    let identity = &identities[0];
    if row_value::<String>(
        identity,
        0,
        PostgresStoreSetupErrorKind::CompatibilityMismatch,
    )? != CODEBASE_MEMORY_EXTENSION_ID
        || row_value::<i16>(
            identity,
            1,
            PostgresStoreSetupErrorKind::CompatibilityMismatch,
        )? != CODEBASE_MEMORY_V2_SCHEMA_VERSION
        || row_value::<String>(
            identity,
            2,
            PostgresStoreSetupErrorKind::CompatibilityMismatch,
        )? != CODEBASE_MEMORY_V2_PATH
        || row_value::<String>(
            identity,
            3,
            PostgresStoreSetupErrorKind::CompatibilityMismatch,
        )? != CODEBASE_MEMORY_V2_SQL_SHA256
        || row_value::<String>(
            identity,
            4,
            PostgresStoreSetupErrorKind::CompatibilityMismatch,
        )? != CODEBASE_MEMORY_V2_MANIFEST_SHA256
        || row_value::<String>(
            identity,
            5,
            PostgresStoreSetupErrorKind::CompatibilityMismatch,
        )? != target.expected_database_uuid()
        || row_value::<String>(
            identity,
            6,
            PostgresStoreSetupErrorKind::CompatibilityMismatch,
        )? != target.expected_database_identity_sha256().as_str()
        || row_value::<i16>(
            identity,
            7,
            PostgresStoreSetupErrorKind::CompatibilityMismatch,
        )? != CODEBASE_MEMORY_V2_GLOBAL_SCHEMA_VERSION
        || row_value::<String>(
            identity,
            8,
            PostgresStoreSetupErrorKind::CompatibilityMismatch,
        )? != CODEBASE_MEMORY_V2_GLOBAL_MANIFEST_SHA256
    {
        return Err(PostgresStoreSetupError::new(
            PostgresStoreSetupErrorKind::CompatibilityMismatch,
        ));
    }

    let ledger = client
        .query(
            "SELECT ledger_ordinal, singleton, extension_id::text, \
                    extension_schema_version, btrim(extension_sql_sha256)::text, \
                    btrim(extension_manifest_sha256)::text, database_uuid::text, \
                    btrim(database_identity_sha256)::text, global_schema_version, \
                    btrim(global_manifest_sha256)::text, event_kind::text \
               FROM ONLY memory.codebase_memory_extension_ledger \
              ORDER BY ledger_ordinal",
            &[],
        )
        .map_err(|error| {
            map_postgres_error(&error, PostgresStoreSetupErrorKind::CompatibilityMismatch)
        })?;
    if ledger.len() != 1 {
        return Err(PostgresStoreSetupError::new(
            PostgresStoreSetupErrorKind::CompatibilityMismatch,
        ));
    }
    let row = &ledger[0];
    if row_value::<i16>(row, 0, PostgresStoreSetupErrorKind::CompatibilityMismatch)? != 1
        || !row_value::<bool>(row, 1, PostgresStoreSetupErrorKind::CompatibilityMismatch)?
        || row_value::<String>(row, 2, PostgresStoreSetupErrorKind::CompatibilityMismatch)?
            != CODEBASE_MEMORY_EXTENSION_ID
        || row_value::<i16>(row, 3, PostgresStoreSetupErrorKind::CompatibilityMismatch)?
            != CODEBASE_MEMORY_V2_SCHEMA_VERSION
        || row_value::<String>(row, 4, PostgresStoreSetupErrorKind::CompatibilityMismatch)?
            != CODEBASE_MEMORY_V2_SQL_SHA256
        || row_value::<String>(row, 5, PostgresStoreSetupErrorKind::CompatibilityMismatch)?
            != CODEBASE_MEMORY_V2_MANIFEST_SHA256
        || row_value::<String>(row, 6, PostgresStoreSetupErrorKind::CompatibilityMismatch)?
            != target.expected_database_uuid()
        || row_value::<String>(row, 7, PostgresStoreSetupErrorKind::CompatibilityMismatch)?
            != target.expected_database_identity_sha256().as_str()
        || row_value::<i16>(row, 8, PostgresStoreSetupErrorKind::CompatibilityMismatch)?
            != CODEBASE_MEMORY_V2_GLOBAL_SCHEMA_VERSION
        || row_value::<String>(row, 9, PostgresStoreSetupErrorKind::CompatibilityMismatch)?
            != CODEBASE_MEMORY_V2_GLOBAL_MANIFEST_SHA256
        || row_value::<String>(row, 10, PostgresStoreSetupErrorKind::CompatibilityMismatch)?
            != "INSTALLED"
    {
        return Err(PostgresStoreSetupError::new(
            PostgresStoreSetupErrorKind::CompatibilityMismatch,
        ));
    }
    Ok(())
}

fn verify_codebase_memory_v3_identity_for_role<C: GenericClient>(
    client: &mut C,
    target: &MigrationTarget,
    global_manifest: &ManifestEvidence,
    role: DatabaseRole,
) -> Result<(), PostgresStoreSetupError> {
    if role == DatabaseRole::Migrator {
        verify_codebase_memory_v3_identity(client, target, global_manifest)?;
    }
    Ok(())
}

#[allow(clippy::too_many_lines)]
fn verify_codebase_memory_v3_identity<C: GenericClient>(
    client: &mut C,
    target: &MigrationTarget,
    global_manifest: &ManifestEvidence,
) -> Result<(), PostgresStoreSetupError> {
    let identities = client
        .query(
            "SELECT extension_id::text, extension_schema_version, extension_path::text, \
                    btrim(extension_sql_sha256)::text, \
                    btrim(extension_manifest_sha256)::text, database_uuid::text, \
                    btrim(database_identity_sha256)::text, global_schema_version, \
                    btrim(global_manifest_sha256)::text \
               FROM ONLY memory.codebase_memory_extension_identity \
              WHERE singleton",
            &[],
        )
        .map_err(|error| {
            map_postgres_error(&error, PostgresStoreSetupErrorKind::CompatibilityMismatch)
        })?;
    if identities.len() != 1 {
        return Err(PostgresStoreSetupError::new(
            PostgresStoreSetupErrorKind::CompatibilityMismatch,
        ));
    }
    let row = &identities[0];
    let extension_id =
        row_value::<String>(row, 0, PostgresStoreSetupErrorKind::CompatibilityMismatch)?;
    let extension_schema_version =
        row_value::<i16>(row, 1, PostgresStoreSetupErrorKind::CompatibilityMismatch)?;
    let extension_path =
        row_value::<String>(row, 2, PostgresStoreSetupErrorKind::CompatibilityMismatch)?;
    let extension_sql_sha256 =
        row_value::<String>(row, 3, PostgresStoreSetupErrorKind::CompatibilityMismatch)?;
    let extension_manifest_sha256 =
        row_value::<String>(row, 4, PostgresStoreSetupErrorKind::CompatibilityMismatch)?;
    let database_uuid =
        row_value::<String>(row, 5, PostgresStoreSetupErrorKind::CompatibilityMismatch)?;
    let database_identity_sha256 =
        row_value::<String>(row, 6, PostgresStoreSetupErrorKind::CompatibilityMismatch)?;
    let global_schema_version =
        row_value::<i16>(row, 7, PostgresStoreSetupErrorKind::CompatibilityMismatch)?;
    let global_manifest_sha256 =
        row_value::<String>(row, 8, PostgresStoreSetupErrorKind::CompatibilityMismatch)?;
    let actual = CodebaseMemoryIdentityProfile {
        extension_id: &extension_id,
        extension_schema_version,
        extension_path: &extension_path,
        extension_sql_sha256: &extension_sql_sha256,
        extension_manifest_sha256: &extension_manifest_sha256,
        database_uuid: &database_uuid,
        database_identity_sha256: &database_identity_sha256,
        global_schema_version,
        global_manifest_sha256: &global_manifest_sha256,
    };
    let expected = CodebaseMemoryIdentityProfile {
        extension_id: CODEBASE_MEMORY_EXTENSION_ID,
        extension_schema_version: CODEBASE_MEMORY_V3_SCHEMA_VERSION,
        extension_path: CODEBASE_MEMORY_V3_PATH,
        extension_sql_sha256: CODEBASE_MEMORY_V3_SQL_SHA256,
        extension_manifest_sha256: CODEBASE_MEMORY_V3_MANIFEST_SHA256,
        database_uuid: target.expected_database_uuid(),
        database_identity_sha256: target.expected_database_identity_sha256().as_str(),
        global_schema_version: CODEBASE_MEMORY_V3_GLOBAL_SCHEMA_VERSION,
        global_manifest_sha256: global_manifest.manifest_sha256().as_str(),
    };
    if !codebase_memory_identity_profile_matches(actual, expected) {
        return Err(PostgresStoreSetupError::new(
            PostgresStoreSetupErrorKind::CompatibilityMismatch,
        ));
    }

    let ledger = client
        .query(
            "SELECT ledger_ordinal, singleton, extension_id::text, \
                    extension_schema_version, btrim(extension_sql_sha256)::text, \
                    btrim(extension_manifest_sha256)::text, database_uuid::text, \
                    btrim(database_identity_sha256)::text, global_schema_version, \
                    btrim(global_manifest_sha256)::text, event_kind::text \
               FROM ONLY memory.codebase_memory_extension_ledger \
              ORDER BY ledger_ordinal",
            &[],
        )
        .map_err(|error| {
            map_postgres_error(&error, PostgresStoreSetupErrorKind::CompatibilityMismatch)
        })?;
    if ledger.len() != 2 {
        return Err(PostgresStoreSetupError::new(
            PostgresStoreSetupErrorKind::CompatibilityMismatch,
        ));
    }
    let expected_ledger = [
        (
            1_i16,
            CODEBASE_MEMORY_V2_SCHEMA_VERSION,
            CODEBASE_MEMORY_V2_SQL_SHA256,
            CODEBASE_MEMORY_V2_MANIFEST_SHA256,
            CODEBASE_MEMORY_V2_GLOBAL_SCHEMA_VERSION,
            CODEBASE_MEMORY_V2_GLOBAL_MANIFEST_SHA256,
            "INSTALLED",
        ),
        (
            2_i16,
            CODEBASE_MEMORY_V3_SCHEMA_VERSION,
            CODEBASE_MEMORY_V3_SQL_SHA256,
            CODEBASE_MEMORY_V3_MANIFEST_SHA256,
            CODEBASE_MEMORY_V3_GLOBAL_SCHEMA_VERSION,
            global_manifest.manifest_sha256().as_str(),
            "UPGRADED",
        ),
    ];
    for (row, expected) in ledger.iter().zip(expected_ledger) {
        if row_value::<i16>(row, 0, PostgresStoreSetupErrorKind::CompatibilityMismatch)?
            != expected.0
            || !row_value::<bool>(row, 1, PostgresStoreSetupErrorKind::CompatibilityMismatch)?
            || row_value::<String>(row, 2, PostgresStoreSetupErrorKind::CompatibilityMismatch)?
                != CODEBASE_MEMORY_EXTENSION_ID
            || row_value::<i16>(row, 3, PostgresStoreSetupErrorKind::CompatibilityMismatch)?
                != expected.1
            || row_value::<String>(row, 4, PostgresStoreSetupErrorKind::CompatibilityMismatch)?
                != expected.2
            || row_value::<String>(row, 5, PostgresStoreSetupErrorKind::CompatibilityMismatch)?
                != expected.3
            || row_value::<String>(row, 6, PostgresStoreSetupErrorKind::CompatibilityMismatch)?
                != target.expected_database_uuid()
            || row_value::<String>(row, 7, PostgresStoreSetupErrorKind::CompatibilityMismatch)?
                != target.expected_database_identity_sha256().as_str()
            || row_value::<i16>(row, 8, PostgresStoreSetupErrorKind::CompatibilityMismatch)?
                != expected.4
            || row_value::<String>(row, 9, PostgresStoreSetupErrorKind::CompatibilityMismatch)?
                != expected.5
            || row_value::<String>(row, 10, PostgresStoreSetupErrorKind::CompatibilityMismatch)?
                != expected.6
        {
            return Err(PostgresStoreSetupError::new(
                PostgresStoreSetupErrorKind::CompatibilityMismatch,
            ));
        }
    }
    Ok(())
}

fn verify_v4_upgrade_source<C: GenericClient>(
    client: &mut C,
    v4_manifest: &ManifestEvidence,
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
             LOCK TABLE control.task_ledger_outbox IN ACCESS EXCLUSIVE MODE; \
             LOCK TABLE control.project_registry_state IN ACCESS EXCLUSIVE MODE; \
             LOCK TABLE control.project_registry_projects IN ACCESS EXCLUSIVE MODE; \
             LOCK TABLE control.project_registry_commands IN ACCESS EXCLUSIVE MODE; \
             LOCK TABLE control.project_registry_identity_reservations IN ACCESS EXCLUSIVE MODE; \
             LOCK TABLE control.project_registry_observations IN ACCESS EXCLUSIVE MODE",
        )
        .map_err(|error| {
            map_postgres_error(&error, PostgresStoreSetupErrorKind::TransactionFailed)
        })?;
    let profile = classify_current_catalog_profile(client, 4)?;
    if profile != CatalogProfile::V4 {
        return Err(PostgresStoreSetupError::new(
            PostgresStoreSetupErrorKind::CompatibilityMismatch,
        ));
    }
    verify_schema_objects(client, profile)?;
    let rows = read_history_rows(client)?;
    verify_history_rows(&rows, &migration_manifest()[..5])?;
    verify_compatibility(client, v4_manifest, profile)?;
    read_database_identity(client, target)?;
    verify_stopped_admission(client)?;
    verify_roles_and_grants(client, profile)
}

fn verify_v5_upgrade_source<C: GenericClient>(
    client: &mut C,
    v5_manifest: &ManifestEvidence,
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
    verify_schema_objects(client, CatalogProfile::V5CodebaseMemoryV3Current)?;
    let rows = read_history_rows(client)?;
    verify_history_rows(&rows, &migration_manifest()[..6])?;
    verify_compatibility(
        client,
        v5_manifest,
        CatalogProfile::V5CodebaseMemoryV3Current,
    )?;
    read_database_identity(client, target)?;
    verify_codebase_memory_v3_identity(client, target, v5_manifest)?;
    verify_stopped_admission(client)?;
    verify_writer_lease_v3_bridge_catalog(client)?;
    verify_roles_and_grants(
        client,
        CatalogProfile::V5CodebaseMemoryV3WriterLeaseV2BridgePending,
    )
}

#[allow(clippy::too_many_lines)]
fn verify_writer_lease_v3_bridge_catalog<C: GenericClient>(
    client: &mut C,
) -> Result<(), PostgresStoreSetupError> {
    if WRITER_LEASE_V3_SQL.len() != 17_568
        || hex_digest(Sha256::digest(WRITER_LEASE_V3_SQL.as_bytes()).as_ref())
            != WRITER_LEASE_V3_SQL_SHA256
        || WRITER_LEASE_V3_REBIND_SQL.is_empty()
    {
        return Err(catalog_error());
    }
    let header = client
        .query_one(
            "SELECT \
             (SELECT count(*) FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace \
               WHERE n.nspname='writer_lease' AND c.relkind='r'), \
             (SELECT count(*) FROM pg_catalog.pg_proc p JOIN pg_catalog.pg_namespace n ON n.oid=p.pronamespace \
               WHERE n.nspname='writer_lease'), \
             (SELECT count(*) FROM pg_catalog.pg_constraint con JOIN pg_catalog.pg_namespace n ON n.oid=con.connamespace \
               WHERE n.nspname='writer_lease'), \
             (SELECT count(*) FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace \
               WHERE n.nspname='writer_lease' AND c.relkind='i'), \
             (SELECT count(*) FROM pg_catalog.pg_proc p JOIN pg_catalog.pg_namespace n ON n.oid=p.pronamespace \
               WHERE n.nspname='writer_lease' AND pg_catalog.has_function_privilege('lattice_runtime',p.oid,'EXECUTE')), \
             pg_catalog.has_schema_privilege('lattice_runtime','writer_lease','USAGE'), \
             (SELECT pg_catalog.obj_description(n.oid,'pg_namespace') FROM pg_catalog.pg_namespace n \
               WHERE n.nspname='writer_lease')",
            &[],
        )
        .map_err(|error| map_postgres_error(&error, PostgresStoreSetupErrorKind::CorruptCatalog))?;
    for (index, expected) in [5_i64, 12, 27, 8, 0].into_iter().enumerate() {
        if row_value::<i64>(&header, index, PostgresStoreSetupErrorKind::CorruptCatalog)?
            != expected
        {
            return Err(catalog_error());
        }
    }
    if row_value::<bool>(&header, 5, PostgresStoreSetupErrorKind::PermissionDenied)?
        || row_value::<Option<String>>(&header, 6, PostgresStoreSetupErrorKind::CorruptCatalog)?
            .as_deref()
            != Some("LATTICE_WRITER_LEASE_SCHEMA_V3")
    {
        return Err(catalog_error());
    }
    for (query, expected, error_kind) in [
        (
            WRITER_LEASE_V1_COLUMN_PROFILE_SQL,
            "560e93c2a765db0024c0e74d25a51b90cfc72b204601139de8fdb688d48c0610",
            PostgresStoreSetupErrorKind::CorruptCatalog,
        ),
        (
            WRITER_LEASE_V1_TABLE_ACL_PROFILE_SQL,
            "b99ef0c0ea5b550ae5e805d29b0020e31c1800a016b0de82cda566d7b25e9569",
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
    ] {
        if catalog_signature(client, query, error_kind)? != expected {
            return Err(PostgresStoreSetupError::new(error_kind));
        }
    }
    verify_writer_lease_v3_functions(client, false)?;
    verify_writer_lease_acl_closure(client, 12, false)
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
        | CatalogProfile::V3CodebaseMemoryV2WriterLeaseV1
        | CatalogProfile::V3CodebaseMemoryV2WriterLeaseV2Bridge => [3, 3, 3, 3, 3],
        CatalogProfile::V4 => [4, 4, 4, 4, 4],
        CatalogProfile::V5
        | CatalogProfile::V5CodebaseMemoryV2UpgradePending
        | CatalogProfile::V5CodebaseMemoryV3Current
        | CatalogProfile::V5CodebaseMemoryV2WriterLeaseV2BridgePending
        | CatalogProfile::V5CodebaseMemoryV3WriterLeaseV2BridgePending
        | CatalogProfile::V5CodebaseMemoryV3WriterLeaseV2Current => [5, 5, 5, 5, 5],
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

#[cfg_attr(not(test), allow(dead_code))]
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

fn retained_history_rows(
    rows: &[Row],
) -> Result<Vec<RetainedMigrationHistoryRow>, PostgresStoreSetupError> {
    rows.iter()
        .map(|row| {
            Ok(RetainedMigrationHistoryRow {
                ordinal: row_value(row, 0, PostgresStoreSetupErrorKind::HistoryMismatch)?,
                migration_id: row_value(row, 1, PostgresStoreSetupErrorKind::HistoryMismatch)?,
                migration_path: row_value(row, 2, PostgresStoreSetupErrorKind::HistoryMismatch)?,
                byte_length: row_value(row, 3, PostgresStoreSetupErrorKind::HistoryMismatch)?,
                checksum_sha256: row_value(row, 4, PostgresStoreSetupErrorKind::HistoryMismatch)?,
                migration_status: row_value(row, 5, PostgresStoreSetupErrorKind::HistoryMismatch)?,
                transaction_mode: row_value(row, 6, PostgresStoreSetupErrorKind::HistoryMismatch)?,
                schema_version: row_value(row, 7, PostgresStoreSetupErrorKind::HistoryMismatch)?,
                min_reader: row_value(row, 8, PostgresStoreSetupErrorKind::HistoryMismatch)?,
                max_reader: row_value(row, 9, PostgresStoreSetupErrorKind::HistoryMismatch)?,
                min_writer: row_value(row, 10, PostgresStoreSetupErrorKind::HistoryMismatch)?,
                max_writer: row_value(row, 11, PostgresStoreSetupErrorKind::HistoryMismatch)?,
            })
        })
        .collect()
}

fn read_retained_schema_compatibility<C: GenericClient>(
    client: &mut C,
) -> Result<RetainedSchemaCompatibility, PostgresStoreSetupError> {
    let rows = client
        .query(
            "SELECT manifest_sha256,current_schema_version,min_reader,max_reader,min_writer,max_writer \
               FROM ONLY control.schema_compatibility WHERE singleton = true",
            &[],
        )
        .map_err(|error| map_postgres_error(&error, PostgresStoreSetupErrorKind::CorruptCatalog))?;
    if rows.len() != 1 {
        return Err(catalog_error());
    }
    let row = &rows[0];
    Ok(RetainedSchemaCompatibility {
        manifest_sha256: row_value(row, 0, PostgresStoreSetupErrorKind::CompatibilityMismatch)?,
        versions: [
            row_value(row, 1, PostgresStoreSetupErrorKind::CompatibilityMismatch)?,
            row_value(row, 2, PostgresStoreSetupErrorKind::CompatibilityMismatch)?,
            row_value(row, 3, PostgresStoreSetupErrorKind::CompatibilityMismatch)?,
            row_value(row, 4, PostgresStoreSetupErrorKind::CompatibilityMismatch)?,
            row_value(row, 5, PostgresStoreSetupErrorKind::CompatibilityMismatch)?,
        ],
    })
}

fn classify_retained_history<C: GenericClient>(
    client: &mut C,
) -> Result<RetainedHistoryClassification, PostgresStoreSetupError> {
    let row = client
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
        .map_err(|error| {
            map_postgres_error(&error, PostgresStoreSetupErrorKind::HistoryMismatch)
        })?;
    classify_retained_history_snapshot(&row)
}

fn classify_retained_history_snapshot(
    row: &Row,
) -> Result<RetainedHistoryClassification, PostgresStoreSetupError> {
    let ordinals: Vec<i16> = row_value(row, 0, PostgresStoreSetupErrorKind::HistoryMismatch)?;
    let migration_ids: Vec<String> =
        row_value(row, 1, PostgresStoreSetupErrorKind::HistoryMismatch)?;
    let migration_paths: Vec<String> =
        row_value(row, 2, PostgresStoreSetupErrorKind::HistoryMismatch)?;
    let byte_lengths: Vec<i64> = row_value(row, 3, PostgresStoreSetupErrorKind::HistoryMismatch)?;
    let checksums: Vec<String> = row_value(row, 4, PostgresStoreSetupErrorKind::HistoryMismatch)?;
    let statuses: Vec<String> = row_value(row, 5, PostgresStoreSetupErrorKind::HistoryMismatch)?;
    let modes: Vec<String> = row_value(row, 6, PostgresStoreSetupErrorKind::HistoryMismatch)?;
    let schema_versions: Vec<i16> =
        row_value(row, 7, PostgresStoreSetupErrorKind::HistoryMismatch)?;
    let min_readers: Vec<i16> = row_value(row, 8, PostgresStoreSetupErrorKind::HistoryMismatch)?;
    let max_readers: Vec<i16> = row_value(row, 9, PostgresStoreSetupErrorKind::HistoryMismatch)?;
    let min_writers: Vec<i16> = row_value(row, 10, PostgresStoreSetupErrorKind::HistoryMismatch)?;
    let max_writers: Vec<i16> = row_value(row, 11, PostgresStoreSetupErrorKind::HistoryMismatch)?;
    let length = ordinals.len();
    if [
        migration_ids.len(),
        migration_paths.len(),
        byte_lengths.len(),
        checksums.len(),
        statuses.len(),
        modes.len(),
        schema_versions.len(),
        min_readers.len(),
        max_readers.len(),
        min_writers.len(),
        max_writers.len(),
    ]
    .into_iter()
    .any(|candidate| candidate != length)
    {
        return Ok(RetainedHistoryClassification::Corrupt);
    }
    let retained = (0..length)
        .map(|index| RetainedMigrationHistoryRow {
            ordinal: ordinals[index],
            migration_id: migration_ids[index].clone(),
            migration_path: migration_paths[index].clone(),
            byte_length: byte_lengths[index],
            checksum_sha256: checksums[index].clone(),
            migration_status: statuses[index].clone(),
            transaction_mode: modes[index].clone(),
            schema_version: schema_versions[index],
            min_reader: min_readers[index],
            max_reader: max_readers[index],
            min_writer: min_writers[index],
            max_writer: max_writers[index],
        })
        .collect::<Vec<_>>();
    let compatibility_values = [
        row_value::<Option<i16>>(row, 13, PostgresStoreSetupErrorKind::CompatibilityMismatch)?,
        row_value::<Option<i16>>(row, 14, PostgresStoreSetupErrorKind::CompatibilityMismatch)?,
        row_value::<Option<i16>>(row, 15, PostgresStoreSetupErrorKind::CompatibilityMismatch)?,
        row_value::<Option<i16>>(row, 16, PostgresStoreSetupErrorKind::CompatibilityMismatch)?,
        row_value::<Option<i16>>(row, 17, PostgresStoreSetupErrorKind::CompatibilityMismatch)?,
    ];
    let Some(versions) = compatibility_values
        .into_iter()
        .collect::<Option<Vec<_>>>()
        .and_then(|values| values.try_into().ok())
    else {
        return Ok(RetainedHistoryClassification::Corrupt);
    };
    let Some(manifest_sha256) =
        row_value::<Option<String>>(row, 12, PostgresStoreSetupErrorKind::CompatibilityMismatch)?
    else {
        return Ok(RetainedHistoryClassification::Corrupt);
    };
    Ok(classify_retained_history_rows(
        &retained,
        &RetainedSchemaCompatibility {
            manifest_sha256,
            versions,
        },
    ))
}

fn classify_retained_history_rows(
    rows: &[RetainedMigrationHistoryRow],
    compatibility: &RetainedSchemaCompatibility,
) -> RetainedHistoryClassification {
    let current = migration_manifest();
    let embedded_prefix_len = rows.len().min(current.len());
    if rows.is_empty()
        || !rows[..embedded_prefix_len]
            .iter()
            .zip(&current[..embedded_prefix_len])
            .all(|(row, expected)| retained_history_matches(row, expected))
    {
        return RetainedHistoryClassification::Corrupt;
    }

    let Some(metadata) = retained_history_metadata(rows) else {
        return RetainedHistoryClassification::Corrupt;
    };
    let retained_digest = migration_metadata_sha256(&metadata);
    if rows.len() <= current.len() {
        let Some(current_schema) = rows.last().map(|row| row.schema_version) else {
            return RetainedHistoryClassification::Corrupt;
        };
        return if compatibility.manifest_sha256 == retained_digest
            && compatibility.versions == [current_schema; 5]
        {
            RetainedHistoryClassification::ExactSupported
        } else {
            RetainedHistoryClassification::Corrupt
        };
    }

    let Ok(mut previous_schema) = i16::try_from(POSTGRES_SCHEMA_VERSION) else {
        return RetainedHistoryClassification::Corrupt;
    };
    for (index, row) in rows.iter().enumerate().skip(current.len()) {
        let Ok(expected_ordinal) = i16::try_from(index + 1) else {
            return RetainedHistoryClassification::Corrupt;
        };
        let Some(expected_schema) = previous_schema.checked_add(1) else {
            return RetainedHistoryClassification::Corrupt;
        };
        let id_prefix = format!("{expected_ordinal:04}_");
        if row.ordinal != expected_ordinal
            || !row.migration_id.starts_with(&id_prefix)
            || !row
                .migration_id
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
            || row.migration_path != format!("db/migrations/{}.sql", row.migration_id)
            || row.byte_length <= 0
            || row.checksum_sha256.len() != 64
            || row
                .checksum_sha256
                .bytes()
                .any(|byte| !byte.is_ascii_digit() && !(b'a'..=b'f').contains(&byte))
            || row.checksum_sha256.bytes().all(|byte| byte == b'0')
            || row.migration_status != MigrationStatus::Executable.as_str()
            || row.transaction_mode != "RUNNER_OWNED"
            || row.schema_version != expected_schema
            || row.min_reader != row.schema_version
            || row.max_reader != row.schema_version
            || row.min_writer != row.schema_version
            || row.max_writer != row.schema_version
        {
            return RetainedHistoryClassification::Corrupt;
        }
        previous_schema = row.schema_version;
    }
    if compatibility.manifest_sha256 == retained_digest
        && compatibility.versions == [previous_schema; 5]
    {
        RetainedHistoryClassification::StrictFutureSuffix
    } else {
        RetainedHistoryClassification::Corrupt
    }
}

fn retained_history_metadata(
    rows: &[RetainedMigrationHistoryRow],
) -> Option<Vec<MigrationMetadata<'_>>> {
    rows.iter()
        .map(|row| {
            let status = match row.migration_status.as_str() {
                "SUPERSEDED" => MigrationStatus::Superseded,
                "EXECUTABLE" => MigrationStatus::Executable,
                _ => return None,
            };
            let transaction_mode = match row.transaction_mode.as_str() {
                "NOT_EXECUTED" => MigrationTransactionMode::NotExecuted,
                "RUNNER_OWNED" => MigrationTransactionMode::RunnerOwned,
                _ => return None,
            };
            Some(MigrationMetadata {
                ordinal: u16::try_from(row.ordinal).ok()?,
                id: &row.migration_id,
                path: &row.migration_path,
                byte_length: u64::try_from(row.byte_length).ok()?,
                sha256: &row.checksum_sha256,
                status,
                transaction_mode,
                schema_version: u16::try_from(row.schema_version).ok()?,
                min_reader: u16::try_from(row.min_reader).ok()?,
                max_reader: u16::try_from(row.max_reader).ok()?,
                min_writer: u16::try_from(row.min_writer).ok()?,
                max_writer: u16::try_from(row.max_writer).ok()?,
            })
        })
        .collect()
}

fn retained_history_matches(
    row: &RetainedMigrationHistoryRow,
    expected: &MigrationDescriptor,
) -> bool {
    let Ok(ordinal) = i16::try_from(expected.ordinal()) else {
        return false;
    };
    let Ok(byte_length) = i64::try_from(expected.byte_length()) else {
        return false;
    };
    let Ok(schema_version) = i16::try_from(expected.schema_version()) else {
        return false;
    };
    let Ok(min_reader) = i16::try_from(*expected.reader_compatibility().start()) else {
        return false;
    };
    let Ok(max_reader) = i16::try_from(*expected.reader_compatibility().end()) else {
        return false;
    };
    let Ok(min_writer) = i16::try_from(*expected.writer_compatibility().start()) else {
        return false;
    };
    let Ok(max_writer) = i16::try_from(*expected.writer_compatibility().end()) else {
        return false;
    };
    row.ordinal == ordinal
        && row.migration_id == expected.id()
        && row.migration_path == expected.path()
        && row.byte_length == byte_length
        && row.checksum_sha256 == expected.sha256()
        && row.migration_status == expected.status().as_str()
        && row.transaction_mode == expected.transaction_mode().as_str()
        && [
            row.schema_version,
            row.min_reader,
            row.max_reader,
            row.min_writer,
            row.max_writer,
        ] == [
            schema_version,
            min_reader,
            max_reader,
            min_writer,
            max_writer,
        ]
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
    verify_schema_objects_with_contract(client, profile, false)
}

fn verify_schema_objects_with_contract<C: GenericClient>(
    client: &mut C,
    profile: CatalogProfile,
    v3_prefix: bool,
) -> Result<(), PostgresStoreSetupError> {
    verify_catalog_signatures(client, profile, v3_prefix)?;
    verify_schema_headers(client, profile)?;
    verify_forbidden_schema_objects(client, profile, v3_prefix)?;
    if matches!(
        profile,
        CatalogProfile::V2
            | CatalogProfile::V3
            | CatalogProfile::V3CodebaseMemoryV2
            | CatalogProfile::V3CodebaseMemoryV2WriterLeaseV1
            | CatalogProfile::V3CodebaseMemoryV2WriterLeaseV2Bridge
            | CatalogProfile::V4
            | CatalogProfile::V5
            | CatalogProfile::V5CodebaseMemoryV2UpgradePending
            | CatalogProfile::V5CodebaseMemoryV3Current
            | CatalogProfile::V5CodebaseMemoryV2WriterLeaseV2BridgePending
            | CatalogProfile::V5CodebaseMemoryV3WriterLeaseV2BridgePending
            | CatalogProfile::V5CodebaseMemoryV3WriterLeaseV2Current
    ) {
        verify_owned_function_boundary(client, profile, v3_prefix)?;
    }
    if matches!(profile, CatalogProfile::V3CodebaseMemoryV2WriterLeaseV1) {
        verify_writer_lease_v1_profile(client)?;
    }
    match profile {
        CatalogProfile::V3CodebaseMemoryV2WriterLeaseV2Bridge
        | CatalogProfile::V5CodebaseMemoryV2WriterLeaseV2BridgePending
        | CatalogProfile::V5CodebaseMemoryV3WriterLeaseV2BridgePending => {
            verify_writer_lease_v2_catalog(client, WriterLeaseV2RuntimeProfile::Bridge)?;
        }
        CatalogProfile::V5CodebaseMemoryV3WriterLeaseV2Current => {
            verify_writer_lease_v2_catalog(client, WriterLeaseV2RuntimeProfile::Current)?;
        }
        CatalogProfile::PreSchema
        | CatalogProfile::V1
        | CatalogProfile::V2
        | CatalogProfile::V3
        | CatalogProfile::V3CodebaseMemoryV2
        | CatalogProfile::V3CodebaseMemoryV2WriterLeaseV1
        | CatalogProfile::V4
        | CatalogProfile::V5
        | CatalogProfile::V5CodebaseMemoryV2UpgradePending
        | CatalogProfile::V5CodebaseMemoryV3Current => {}
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
                JOIN pg_catalog.pg_namespace n ON n.oid=x.tmplnamespace WHERE n.nspname='writer_lease') + \
              (SELECT pg_catalog.count(*) FROM pg_catalog.pg_cast c \
                WHERE c.castsource IN (SELECT t.oid FROM pg_catalog.pg_type t \
                      JOIN pg_catalog.pg_namespace n ON n.oid=t.typnamespace \
                     WHERE n.nspname='writer_lease') \
                   OR c.casttarget IN (SELECT t.oid FROM pg_catalog.pg_type t \
                      JOIN pg_catalog.pg_namespace n ON n.oid=t.typnamespace \
                     WHERE n.nspname='writer_lease') \
                   OR c.castfunc IN (SELECT p.oid FROM pg_catalog.pg_proc p \
                      JOIN pg_catalog.pg_namespace n ON n.oid=p.pronamespace \
                     WHERE n.nspname='writer_lease')) + \
              (SELECT pg_catalog.count(*) FROM pg_catalog.pg_transform tr \
                WHERE tr.trftype IN (SELECT t.oid FROM pg_catalog.pg_type t \
                      JOIN pg_catalog.pg_namespace n ON n.oid=t.typnamespace \
                     WHERE n.nspname='writer_lease') \
                   OR tr.trffromsql IN (SELECT p.oid FROM pg_catalog.pg_proc p \
                      JOIN pg_catalog.pg_namespace n ON n.oid=p.pronamespace \
                     WHERE n.nspname='writer_lease') \
                   OR tr.trftosql IN (SELECT p.oid FROM pg_catalog.pg_proc p \
                      JOIN pg_catalog.pg_namespace n ON n.oid=p.pronamespace \
                     WHERE n.nspname='writer_lease'))), \
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

#[allow(clippy::too_many_lines)]
fn verify_writer_lease_v2_catalog<C: GenericClient>(
    client: &mut C,
    runtime: WriterLeaseV2RuntimeProfile,
) -> Result<(), PostgresStoreSetupError> {
    if WRITER_LEASE_V1_SQL.len() != 44_366
        || hex_digest(Sha256::digest(WRITER_LEASE_V1_SQL.as_bytes()).as_ref())
            != WRITER_LEASE_V1_SQL_SHA256
        || WRITER_LEASE_V2_SQL.len() != 22_985
        || hex_digest(Sha256::digest(WRITER_LEASE_V2_SQL.as_bytes()).as_ref())
            != WRITER_LEASE_V2_SQL_SHA256
    {
        return Err(catalog_error());
    }
    let catalog_profiles = match runtime {
        WriterLeaseV2RuntimeProfile::Bridge => &WRITER_LEASE_V2_BRIDGE_CATALOG_PROFILES,
        WriterLeaseV2RuntimeProfile::Current => &WRITER_LEASE_V2_CURRENT_CATALOG_PROFILES,
    };
    for &(query, expected, error_kind) in catalog_profiles {
        if catalog_signature(client, query, error_kind)? != expected {
            return Err(PostgresStoreSetupError::new(error_kind));
        }
    }
    let expected_runtime_functions = match runtime {
        WriterLeaseV2RuntimeProfile::Bridge => 0_i64,
        WriterLeaseV2RuntimeProfile::Current => 7_i64,
    };
    let expected_runtime_usage = runtime == WriterLeaseV2RuntimeProfile::Current;
    let header = client
        .query_one(
            "SELECT \
             (SELECT count(*) FROM pg_catalog.pg_namespace n \
               JOIN pg_catalog.pg_roles owner ON owner.oid=n.nspowner \
              WHERE n.nspname='writer_lease' AND owner.rolname='lattice_migrator'), \
             (SELECT count(*) FROM pg_catalog.pg_class c \
               JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace \
               JOIN pg_catalog.pg_roles owner ON owner.oid=c.relowner \
              WHERE n.nspname='writer_lease' AND c.relkind IN ('r','p') \
                AND owner.rolname='lattice_migrator'), \
             (SELECT count(*) FROM pg_catalog.pg_proc p \
               JOIN pg_catalog.pg_namespace n ON n.oid=p.pronamespace \
               JOIN pg_catalog.pg_roles owner ON owner.oid=p.proowner \
              WHERE n.nspname='writer_lease' AND p.prosecdef \
                AND owner.rolname='lattice_migrator'), \
             (SELECT count(*) FROM pg_catalog.pg_constraint con \
               JOIN pg_catalog.pg_namespace n ON n.oid=con.connamespace \
              WHERE n.nspname='writer_lease'), \
             (SELECT count(*) FROM pg_catalog.pg_class c \
               JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace \
              WHERE n.nspname='writer_lease' AND c.relkind='i'), \
             (SELECT count(*) FROM pg_catalog.pg_proc p \
               JOIN pg_catalog.pg_namespace n ON n.oid=p.pronamespace \
              WHERE n.nspname='writer_lease' \
                AND pg_catalog.has_function_privilege('lattice_runtime',p.oid,'EXECUTE')), \
             (SELECT count(*) FROM pg_catalog.pg_class c \
               JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace \
              WHERE n.nspname='writer_lease' AND c.relkind IN ('r','p') \
                AND pg_catalog.has_table_privilege('lattice_runtime',c.oid, \
                    'SELECT,INSERT,UPDATE,DELETE,TRUNCATE,REFERENCES,TRIGGER,MAINTAIN')), \
             pg_catalog.has_schema_privilege('lattice_runtime','writer_lease','USAGE'), \
             pg_catalog.has_schema_privilege('lattice_runtime','writer_lease','CREATE'), \
             (SELECT pg_catalog.obj_description(n.oid,'pg_namespace') \
                FROM pg_catalog.pg_namespace n WHERE n.nspname='writer_lease')",
            &[],
        )
        .map_err(|error| map_postgres_error(&error, PostgresStoreSetupErrorKind::CorruptCatalog))?;
    for (index, expected) in [1_i64, 5, 9, 27, 8, expected_runtime_functions, 0]
        .into_iter()
        .enumerate()
    {
        if row_value::<i64>(&header, index, PostgresStoreSetupErrorKind::CorruptCatalog)?
            != expected
        {
            return Err(catalog_error());
        }
    }
    if row_value::<bool>(&header, 7, PostgresStoreSetupErrorKind::PermissionDenied)?
        != expected_runtime_usage
        || row_value::<bool>(&header, 8, PostgresStoreSetupErrorKind::PermissionDenied)?
        || row_value::<Option<String>>(&header, 9, PostgresStoreSetupErrorKind::CorruptCatalog)?
            .as_deref()
            != Some("LATTICE_WRITER_LEASE_SCHEMA_V2")
    {
        return Err(permission_error());
    }

    let relations = client
        .query(
            "SELECT c.relname::text || '|' || c.relkind::text || '|' || owner.rolname || '|' \
                    || c.relpersistence::text || '|' || c.relrowsecurity::text || '|' \
                    || c.relforcerowsecurity::text || '|' || c.relhassubclass::text || '|' \
                    || c.relispartition::text || '|' || c.relreplident::text || '|' \
                    || COALESCE(pg_catalog.array_to_string(c.reloptions,','),'<NULL>') || '|' \
                    || COALESCE(pg_catalog.obj_description(c.oid,'pg_class'),'<NULL>') \
               FROM pg_catalog.pg_class c \
               JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace \
               JOIN pg_catalog.pg_roles owner ON owner.oid=c.relowner \
              WHERE n.nspname='writer_lease' AND c.relkind IN ('r','p') \
              ORDER BY c.relname",
            &[],
        )
        .map_err(|error| map_postgres_error(&error, PostgresStoreSetupErrorKind::CorruptCatalog))?
        .into_iter()
        .map(|row| row_value::<String>(&row, 0, PostgresStoreSetupErrorKind::CorruptCatalog))
        .collect::<Result<Vec<_>, _>>()?;
    let expected_relations = [
        "writer_lease_commands|r|lattice_migrator|p|false|false|false|false|d|<NULL>|LATTICE_WRITER_LEASE_COMMANDS_V1",
        "writer_lease_extension_identity|r|lattice_migrator|p|false|false|false|false|d|<NULL>|LATTICE_WRITER_LEASE_EXTENSION_IDENTITY_V2",
        "writer_lease_extension_ledger|r|lattice_migrator|p|false|false|false|false|d|<NULL>|LATTICE_WRITER_LEASE_EXTENSION_LEDGER_V2",
        "writer_lease_heads|r|lattice_migrator|p|false|false|false|false|d|<NULL>|LATTICE_WRITER_LEASE_HEADS_V1",
        "writer_lease_transitions|r|lattice_migrator|p|false|false|false|false|d|<NULL>|LATTICE_WRITER_LEASE_TRANSITIONS_V1",
    ];
    if relations.iter().map(String::as_str).ne(expected_relations) {
        return Err(catalog_error());
    }

    let constraints = client
        .query(
            "SELECT c.relname::text || '.' || con.conname || '|' || con.contype::text \
               FROM pg_catalog.pg_constraint con \
               JOIN pg_catalog.pg_namespace n ON n.oid=con.connamespace \
               JOIN pg_catalog.pg_class c ON c.oid=con.conrelid \
              WHERE n.nspname='writer_lease' \
              ORDER BY c.relname,con.conname",
            &[],
        )
        .map_err(|error| map_postgres_error(&error, PostgresStoreSetupErrorKind::CorruptCatalog))?
        .into_iter()
        .map(|row| row_value::<String>(&row, 0, PostgresStoreSetupErrorKind::CorruptCatalog))
        .collect::<Result<Vec<_>, _>>()?;
    let expected_constraints = [
        "writer_lease_commands.writer_lease_commands_bytes|c",
        "writer_lease_commands.writer_lease_commands_digests|c",
        "writer_lease_commands.writer_lease_commands_id|c",
        "writer_lease_commands.writer_lease_commands_id_unique|u",
        "writer_lease_commands.writer_lease_commands_ordinal|c",
        "writer_lease_commands.writer_lease_commands_outcome|c",
        "writer_lease_commands.writer_lease_commands_pkey|p",
        "writer_lease_commands.writer_lease_commands_project_fk|f",
        "writer_lease_commands.writer_lease_commands_receipt_unique|u",
        "writer_lease_extension_identity.writer_lease_extension_identity_hashes|c",
        "writer_lease_extension_identity.writer_lease_extension_identity_pkey|p",
        "writer_lease_extension_identity.writer_lease_extension_identity_profile|c",
        "writer_lease_extension_identity.writer_lease_extension_identity_singleton|c",
        "writer_lease_extension_ledger.writer_lease_extension_ledger_identity_fk|f",
        "writer_lease_extension_ledger.writer_lease_extension_ledger_pkey|p",
        "writer_lease_extension_ledger.writer_lease_extension_ledger_profile|c",
        "writer_lease_extension_ledger.writer_lease_extension_ledger_singleton|c",
        "writer_lease_heads.writer_lease_heads_command_tail|c",
        "writer_lease_heads.writer_lease_heads_current_closed|c",
        "writer_lease_heads.writer_lease_heads_pkey|p",
        "writer_lease_heads.writer_lease_heads_project|c",
        "writer_lease_heads.writer_lease_heads_snapshot|c",
        "writer_lease_heads.writer_lease_heads_versions|c",
        "writer_lease_transitions.writer_lease_transitions_command_fk|f",
        "writer_lease_transitions.writer_lease_transitions_digest_unique|u",
        "writer_lease_transitions.writer_lease_transitions_identity|c",
        "writer_lease_transitions.writer_lease_transitions_pkey|p",
    ];
    if constraints
        .iter()
        .map(String::as_str)
        .ne(expected_constraints)
    {
        return Err(catalog_error());
    }

    let indexes = client
        .query(
            "SELECT ix.relname::text FROM pg_catalog.pg_index i \
               JOIN pg_catalog.pg_class ix ON ix.oid=i.indexrelid \
               JOIN pg_catalog.pg_class tbl ON tbl.oid=i.indrelid \
               JOIN pg_catalog.pg_namespace n ON n.oid=tbl.relnamespace \
               JOIN pg_catalog.pg_roles owner ON owner.oid=ix.relowner \
              WHERE n.nspname='writer_lease' AND owner.rolname='lattice_migrator' \
                AND i.indisvalid AND i.indisready AND i.indislive \
                AND NOT i.indisclustered AND NOT i.indisreplident \
              ORDER BY ix.relname",
            &[],
        )
        .map_err(|error| map_postgres_error(&error, PostgresStoreSetupErrorKind::CorruptCatalog))?
        .into_iter()
        .map(|row| row_value::<String>(&row, 0, PostgresStoreSetupErrorKind::CorruptCatalog))
        .collect::<Result<Vec<_>, _>>()?;
    let expected_indexes = [
        "writer_lease_commands_id_unique",
        "writer_lease_commands_pkey",
        "writer_lease_commands_receipt_unique",
        "writer_lease_extension_identity_pkey",
        "writer_lease_extension_ledger_pkey",
        "writer_lease_heads_pkey",
        "writer_lease_transitions_digest_unique",
        "writer_lease_transitions_pkey",
    ];
    if indexes.iter().map(String::as_str).ne(expected_indexes) {
        return Err(catalog_error());
    }

    for (query, expected, error_kind) in [
        (
            WRITER_LEASE_V1_COLUMN_PROFILE_SQL,
            "560e93c2a765db0024c0e74d25a51b90cfc72b204601139de8fdb688d48c0610",
            PostgresStoreSetupErrorKind::CorruptCatalog,
        ),
        (
            WRITER_LEASE_V1_TABLE_ACL_PROFILE_SQL,
            "b99ef0c0ea5b550ae5e805d29b0020e31c1800a016b0de82cda566d7b25e9569",
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
    ] {
        if catalog_signature(client, query, error_kind)? != expected {
            return Err(PostgresStoreSetupError::new(error_kind));
        }
    }

    verify_writer_lease_v2_function_catalog(client, runtime)?;
    verify_writer_lease_v2_function_sources(client)?;
    verify_writer_lease_v2_acl_closure(client, runtime)?;
    Ok(())
}

fn verify_writer_lease_exact_catalog_profile<C: GenericClient>(
    client: &mut C,
    expected: &[&str; 10],
) -> Result<(), PostgresStoreSetupError> {
    for ((query, error_kind), expected_signature) in [
        (
            WRITER_LEASE_V1_RELATION_PROFILE_SQL,
            PostgresStoreSetupErrorKind::CorruptCatalog,
        ),
        (
            WRITER_LEASE_V1_COLUMN_PROFILE_SQL,
            PostgresStoreSetupErrorKind::CorruptCatalog,
        ),
        (
            WRITER_LEASE_V1_CONSTRAINT_PROFILE_SQL,
            PostgresStoreSetupErrorKind::CorruptCatalog,
        ),
        (
            WRITER_LEASE_V1_INDEX_PROFILE_SQL,
            PostgresStoreSetupErrorKind::CorruptCatalog,
        ),
        (
            WRITER_LEASE_V1_FUNCTION_PROFILE_SQL,
            PostgresStoreSetupErrorKind::CorruptCatalog,
        ),
        (
            WRITER_LEASE_V1_SCHEMA_ACL_PROFILE_SQL,
            PostgresStoreSetupErrorKind::PermissionDenied,
        ),
        (
            WRITER_LEASE_V1_TABLE_ACL_PROFILE_SQL,
            PostgresStoreSetupErrorKind::PermissionDenied,
        ),
        (
            WRITER_LEASE_V1_FUNCTION_ACL_PROFILE_SQL,
            PostgresStoreSetupErrorKind::PermissionDenied,
        ),
        (
            WRITER_LEASE_V1_COLUMN_ACL_PROFILE_SQL,
            PostgresStoreSetupErrorKind::PermissionDenied,
        ),
        (
            WRITER_LEASE_V1_TYPE_PROFILE_SQL,
            PostgresStoreSetupErrorKind::CorruptCatalog,
        ),
    ]
    .into_iter()
    .zip(expected)
    {
        if catalog_signature(client, query, error_kind)? != *expected_signature {
            return Err(PostgresStoreSetupError::new(error_kind));
        }
    }
    Ok(())
}

fn classify_writer_lease_v5_runtime_profile<C: GenericClient>(
    client: &mut C,
) -> Result<WriterLeaseV5RuntimeProfile, PostgresStoreSetupError> {
    let observed = catalog_signature(
        client,
        WRITER_LEASE_V1_FUNCTION_PROFILE_SQL,
        PostgresStoreSetupErrorKind::CorruptCatalog,
    )?;
    match observed.as_str() {
        value if value == WRITER_LEASE_V5_CURRENT_CATALOG_SIGNATURES[4] => {
            Ok(WriterLeaseV5RuntimeProfile::StoreV7Base)
        }
        value if value == WRITER_LEASE_V5_STORE_V8_CURRENT_CATALOG_SIGNATURES[4] => {
            Ok(WriterLeaseV5RuntimeProfile::StoreV8Successor)
        }
        _ => Err(catalog_error()),
    }
}

fn verify_writer_lease_v5_store_v8_successor<C: GenericClient>(
    client: &mut C,
) -> Result<(), PostgresStoreSetupError> {
    if classify_writer_lease_v5_runtime_profile(client)?
        != WriterLeaseV5RuntimeProfile::StoreV8Successor
    {
        return Err(catalog_error());
    }
    verify_writer_lease_exact_catalog_profile(
        client,
        &WRITER_LEASE_V5_STORE_V8_CURRENT_CATALOG_SIGNATURES,
    )?;
    verify_writer_lease_v5_functions(client, WriterLeaseV5RuntimeProfile::StoreV8Successor)?;
    verify_writer_lease_acl_closure(client, 10, true)
}

fn verify_writer_lease_v2_function_catalog<C: GenericClient>(
    client: &mut C,
    runtime: WriterLeaseV2RuntimeProfile,
) -> Result<(), PostgresStoreSetupError> {
    let observed = client
        .query(
            "SELECT p.proname::text || '(' || pg_catalog.oidvectortypes(p.proargtypes) \
                    || ')|' || p.provolatile::text || '|' || p.proparallel::text || '|' \
                    || pg_catalog.has_function_privilege('lattice_runtime',p.oid,'EXECUTE')::text \
               FROM pg_catalog.pg_proc p \
               JOIN pg_catalog.pg_namespace n ON n.oid=p.pronamespace \
               JOIN pg_catalog.pg_roles owner ON owner.oid=p.proowner \
               JOIN pg_catalog.pg_language language ON language.oid=p.prolang \
              WHERE n.nspname='writer_lease' AND owner.rolname='lattice_migrator' \
                AND language.lanname='plpgsql' AND p.prokind='f' AND p.prosecdef \
                AND NOT p.proleakproof AND NOT p.proisstrict \
                AND p.pronargdefaults=0 \
                AND pg_catalog.array_to_string(p.proconfig,',') = \
                    'search_path=pg_catalog,row_security=on,lock_timeout=5s,statement_timeout=30s' \
              ORDER BY p.proname",
            &[],
        )
        .map_err(|error| map_postgres_error(&error, PostgresStoreSetupErrorKind::CorruptCatalog))?
        .into_iter()
        .map(|row| row_value::<String>(&row, 0, PostgresStoreSetupErrorKind::CorruptCatalog))
        .collect::<Result<Vec<_>, _>>()?;
    let allowed = |name: &str| {
        runtime == WriterLeaseV2RuntimeProfile::Current
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
    if observed != expected {
        return Err(catalog_error());
    }
    Ok(())
}

fn verify_writer_lease_v2_function_sources<C: GenericClient>(
    client: &mut C,
) -> Result<(), PostgresStoreSetupError> {
    let descriptors = [
        (
            "writer_lease_assert_current_v1",
            "lattice_writer_lease_assert_current_v1",
            WRITER_LEASE_V1_SQL,
        ),
        (
            "writer_lease_bind_runtime_v1",
            "lattice_writer_lease_bind_runtime_v1",
            WRITER_LEASE_V1_SQL,
        ),
        (
            "writer_lease_bind_runtime_v2",
            "lattice_writer_lease_bind_runtime_v2",
            WRITER_LEASE_V2_SQL,
        ),
        (
            "writer_lease_commit_plan_v1",
            "lattice_writer_lease_commit_plan_v1",
            WRITER_LEASE_V1_SQL,
        ),
        (
            "writer_lease_load_commands_v1",
            "lattice_writer_lease_load_commands_v1",
            WRITER_LEASE_V1_SQL,
        ),
        (
            "writer_lease_load_current_v1",
            "lattice_writer_lease_load_current_v1",
            WRITER_LEASE_V1_SQL,
        ),
        (
            "writer_lease_load_for_update_v1",
            "lattice_writer_lease_load_for_update_v1",
            WRITER_LEASE_V1_SQL,
        ),
        (
            "writer_lease_load_for_update_v2",
            "lattice_writer_lease_load_for_update_v2",
            WRITER_LEASE_V2_SQL,
        ),
        (
            "writer_lease_load_transitions_v1",
            "lattice_writer_lease_load_transitions_v1",
            WRITER_LEASE_V1_SQL,
        ),
    ];
    let rows = client
        .query(
            "SELECT p.proname::text,p.prosrc::text FROM pg_catalog.pg_proc p \
               JOIN pg_catalog.pg_namespace n ON n.oid=p.pronamespace \
              WHERE n.nspname='writer_lease' ORDER BY p.proname",
            &[],
        )
        .map_err(|error| map_postgres_error(&error, PostgresStoreSetupErrorKind::CorruptCatalog))?;
    if rows.len() != descriptors.len() {
        return Err(catalog_error());
    }
    for (row, (name, delimiter, sql)) in rows.iter().zip(descriptors) {
        if row_value::<String>(row, 0, PostgresStoreSetupErrorKind::CorruptCatalog)? != name
            || row_value::<String>(row, 1, PostgresStoreSetupErrorKind::CorruptCatalog)?
                != embedded_writer_function_source(sql, delimiter)?
        {
            return Err(catalog_error());
        }
    }
    Ok(())
}

fn embedded_writer_function_source<'a>(
    sql: &'a str,
    delimiter: &str,
) -> Result<&'a str, PostgresStoreSetupError> {
    let open = format!("AS ${delimiter}$");
    let close = format!("${delimiter}$;");
    let start = sql.find(&open).ok_or_else(catalog_error)? + open.len();
    let remainder = &sql[start..];
    let end = remainder.find(&close).ok_or_else(catalog_error)?;
    Ok(&remainder[..end])
}

#[allow(clippy::too_many_lines)]
fn verify_writer_lease_v3_functions<C: GenericClient>(
    client: &mut C,
    current: bool,
) -> Result<(), PostgresStoreSetupError> {
    let rows = client
        .query(
            "SELECT p.proname::text,p.prokind::text,l.lanname,r.rolname,p.prosecdef, \
                    p.provolatile::text,p.proparallel::text, \
                    pg_catalog.oidvectortypes(p.proargtypes), \
                    COALESCE(pg_catalog.array_to_string(p.proconfig,','),'<NULL>'), \
                    pg_catalog.has_function_privilege('lattice_runtime',p.oid,'EXECUTE'), \
                    p.prosrc::text,COALESCE(pg_catalog.obj_description(p.oid,'pg_proc'),'<NULL>') \
               FROM pg_catalog.pg_proc p \
               JOIN pg_catalog.pg_namespace n ON n.oid=p.pronamespace \
               JOIN pg_catalog.pg_language l ON l.oid=p.prolang \
               JOIN pg_catalog.pg_roles r ON r.oid=p.proowner \
              WHERE n.nspname='writer_lease' \
              ORDER BY p.proname,pg_catalog.pg_get_function_identity_arguments(p.oid)",
            &[],
        )
        .map_err(|error| map_postgres_error(&error, PostgresStoreSetupErrorKind::CorruptCatalog))?;
    let allowed = |name: &str| {
        current
            && matches!(
                name,
                "writer_lease_assert_current_v1"
                    | "writer_lease_bind_runtime_v3"
                    | "writer_lease_commit_plan_v1"
                    | "writer_lease_load_commands_v1"
                    | "writer_lease_load_current_v1"
                    | "writer_lease_load_for_update_v3"
                    | "writer_lease_load_transitions_v1"
            )
    };
    let expected = [
        (
            "writer_lease_assert_current_v1",
            "f",
            "s",
            "s",
            "text, text, text, text, bytea, text, text, text, text, bigint, bytea, text, bigint, bigint, bytea",
            true,
        ),
        (
            "writer_lease_bind_runtime_v1",
            "f",
            "s",
            "s",
            "text, bigint, bytea, text, text, text, text, text",
            true,
        ),
        (
            "writer_lease_bind_runtime_v2",
            "f",
            "s",
            "s",
            "text, bigint, bytea, text, text, text, text, text",
            true,
        ),
        (
            "writer_lease_bind_runtime_v3",
            "f",
            "s",
            "s",
            "text, bigint, bytea, text, text, text, text, text",
            true,
        ),
        (
            "writer_lease_commit_plan_v1",
            "f",
            "v",
            "u",
            "text, bigint, bytea, bigint, bytea, text, bytea, text, text, bigint, bytea, bytea, bytea, bytea, bigint, bigint, bigint, bytea, text, bytea, text, text, text, bytea, text, text, text, text, bigint, bytea, text, bigint, bigint, text, bigint, text, bytea, bytea, bytea, bytea, bytea, text, text, bytea, bytea, bytea, text, bytea",
            true,
        ),
        ("writer_lease_load_commands_v1", "f", "s", "s", "text", true),
        ("writer_lease_load_current_v1", "f", "s", "s", "text", true),
        (
            "writer_lease_load_for_update_v1",
            "f",
            "v",
            "u",
            "text, bytea, bytea, bytea, text",
            true,
        ),
        (
            "writer_lease_load_for_update_v2",
            "f",
            "v",
            "u",
            "text, bytea, bytea, bytea, text",
            true,
        ),
        (
            "writer_lease_load_for_update_v3",
            "f",
            "v",
            "u",
            "text, bytea, bytea, bytea, text",
            true,
        ),
        (
            "writer_lease_load_transitions_v1",
            "f",
            "s",
            "s",
            "text",
            true,
        ),
        ("writer_lease_rebind_v3", "p", "v", "u", "", false),
    ];
    if rows.len() != expected.len() {
        return Err(catalog_error());
    }
    for (row, (name, kind, volatility, parallel, args, security_definer)) in
        rows.iter().zip(expected)
    {
        if row_value::<String>(row, 0, PostgresStoreSetupErrorKind::CorruptCatalog)? != name
            || row_value::<String>(row, 1, PostgresStoreSetupErrorKind::CorruptCatalog)? != kind
            || row_value::<String>(row, 2, PostgresStoreSetupErrorKind::CorruptCatalog)?
                != "plpgsql"
            || row_value::<String>(row, 3, PostgresStoreSetupErrorKind::CorruptCatalog)?
                != "lattice_migrator"
            || row_value::<bool>(row, 4, PostgresStoreSetupErrorKind::CorruptCatalog)?
                != security_definer
            || row_value::<String>(row, 5, PostgresStoreSetupErrorKind::CorruptCatalog)?
                != volatility
            || row_value::<String>(row, 6, PostgresStoreSetupErrorKind::CorruptCatalog)? != parallel
            || row_value::<String>(row, 7, PostgresStoreSetupErrorKind::CorruptCatalog)? != args
            || row_value::<String>(row, 8, PostgresStoreSetupErrorKind::CorruptCatalog)?
                != "search_path=pg_catalog,row_security=on,lock_timeout=5s,statement_timeout=30s"
            || row_value::<bool>(row, 9, PostgresStoreSetupErrorKind::PermissionDenied)?
                != allowed(name)
        {
            return Err(catalog_error());
        }
        let (sql, delimiter, comment) = match name {
            "writer_lease_bind_runtime_v2" | "writer_lease_load_for_update_v2" => (
                WRITER_LEASE_V2_SQL,
                if name == "writer_lease_bind_runtime_v2" {
                    "lattice_writer_lease_bind_runtime_v2"
                } else {
                    "lattice_writer_lease_load_for_update_v2"
                },
                None,
            ),
            "writer_lease_bind_runtime_v3" | "writer_lease_load_for_update_v3" => (
                WRITER_LEASE_V3_SQL,
                if name == "writer_lease_bind_runtime_v3" {
                    "lattice_writer_lease_bind_runtime_v3"
                } else {
                    "lattice_writer_lease_load_for_update_v3"
                },
                if name == "writer_lease_bind_runtime_v3" {
                    Some("TASK087_GLOBAL_SCHEMA_V6_FOREMAN_COORDINATION_FOREMAN_SNAPSHOT_RECORDED")
                } else {
                    None
                },
            ),
            "writer_lease_rebind_v3" => (
                WRITER_LEASE_V3_REBIND_SQL,
                "lattice_writer_lease_rebind_v3",
                Some("LATTICE_WRITER_LEASE_REBIND_V3"),
            ),
            _ => (
                WRITER_LEASE_V1_SQL,
                match name {
                    "writer_lease_assert_current_v1" => "lattice_writer_lease_assert_current_v1",
                    "writer_lease_bind_runtime_v1" => "lattice_writer_lease_bind_runtime_v1",
                    "writer_lease_commit_plan_v1" => "lattice_writer_lease_commit_plan_v1",
                    "writer_lease_load_commands_v1" => "lattice_writer_lease_load_commands_v1",
                    "writer_lease_load_current_v1" => "lattice_writer_lease_load_current_v1",
                    "writer_lease_load_for_update_v1" => "lattice_writer_lease_load_for_update_v1",
                    "writer_lease_load_transitions_v1" => {
                        "lattice_writer_lease_load_transitions_v1"
                    }
                    _ => return Err(catalog_error()),
                },
                None,
            ),
        };
        if row_value::<String>(row, 10, PostgresStoreSetupErrorKind::CorruptCatalog)?
            != embedded_writer_function_source(sql, delimiter)?
            || row_value::<String>(row, 11, PostgresStoreSetupErrorKind::CorruptCatalog)?
                != comment.unwrap_or("<NULL>")
        {
            return Err(catalog_error());
        }
    }
    let closure = client
        .query_one(
            "SELECT \
             (SELECT count(*) FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace \
               WHERE n.nspname='writer_lease' AND pg_catalog.has_table_privilege(
                 'lattice_runtime',c.oid,'SELECT,INSERT,UPDATE,DELETE,TRUNCATE,REFERENCES,TRIGGER,MAINTAIN')), \
             (SELECT count(*) FROM pg_catalog.pg_proc p JOIN pg_catalog.pg_namespace n ON n.oid=p.pronamespace \
               CROSS JOIN pg_catalog.pg_roles r WHERE n.nspname='writer_lease' AND NOT r.rolsuper \
                 AND r.rolname !~ '^pg_' AND r.rolname NOT IN ('lattice_migrator','lattice_runtime') \
                 AND pg_catalog.has_function_privilege(r.rolname,p.oid,'EXECUTE')), \
             pg_catalog.has_schema_privilege('lattice_runtime','writer_lease','CREATE')",
            &[],
        )
        .map_err(|error| map_postgres_error(&error, PostgresStoreSetupErrorKind::PermissionDenied))?;
    if row_value::<i64>(&closure, 0, PostgresStoreSetupErrorKind::PermissionDenied)? != 0
        || row_value::<i64>(&closure, 1, PostgresStoreSetupErrorKind::PermissionDenied)? != 0
        || row_value::<bool>(&closure, 2, PostgresStoreSetupErrorKind::PermissionDenied)?
    {
        return Err(permission_error());
    }
    Ok(())
}

#[allow(clippy::too_many_lines)]
fn verify_writer_lease_v4_functions<C: GenericClient>(
    client: &mut C,
    current: bool,
) -> Result<(), PostgresStoreSetupError> {
    if WRITER_LEASE_V4_SQL.len() != 19_205
        || hex_digest(Sha256::digest(WRITER_LEASE_V4_SQL.as_bytes()).as_ref())
            != WRITER_LEASE_V4_SQL_SHA256
        || WRITER_LEASE_V4_REBIND_SQL.is_empty()
    {
        return Err(catalog_error());
    }
    let rows = client
        .query(
            "SELECT p.proname::text,p.prokind::text,l.lanname,r.rolname,p.prosecdef, \
                    p.provolatile::text,p.proparallel::text, \
                    pg_catalog.oidvectortypes(p.proargtypes), \
                    COALESCE(pg_catalog.array_to_string(p.proconfig,','),'<NULL>'), \
                    pg_catalog.has_function_privilege('lattice_runtime',p.oid,'EXECUTE'), \
                    p.prosrc::text,COALESCE(pg_catalog.obj_description(p.oid,'pg_proc'),'<NULL>') \
               FROM pg_catalog.pg_proc p \
               JOIN pg_catalog.pg_namespace n ON n.oid=p.pronamespace \
               JOIN pg_catalog.pg_language l ON l.oid=p.prolang \
               JOIN pg_catalog.pg_roles r ON r.oid=p.proowner \
              WHERE n.nspname='writer_lease' \
              ORDER BY p.proname,pg_catalog.pg_get_function_identity_arguments(p.oid)",
            &[],
        )
        .map_err(|error| map_postgres_error(&error, PostgresStoreSetupErrorKind::CorruptCatalog))?;
    let allowed = |name: &str| {
        current
            && matches!(
                name,
                "writer_lease_assert_current_v1"
                    | "writer_lease_bind_runtime_v4"
                    | "writer_lease_commit_plan_v1"
                    | "writer_lease_load_commands_v1"
                    | "writer_lease_load_current_v1"
                    | "writer_lease_load_for_update_v4"
                    | "writer_lease_load_transitions_v1"
            )
    };
    let expected = [
        (
            "writer_lease_assert_current_v1",
            "f",
            "s",
            "s",
            "text, text, text, text, bytea, text, text, text, text, bigint, bytea, text, bigint, bigint, bytea",
            true,
        ),
        (
            "writer_lease_bind_runtime_v1",
            "f",
            "s",
            "s",
            "text, bigint, bytea, text, text, text, text, text",
            true,
        ),
        (
            "writer_lease_bind_runtime_v2",
            "f",
            "s",
            "s",
            "text, bigint, bytea, text, text, text, text, text",
            true,
        ),
        (
            "writer_lease_bind_runtime_v3",
            "f",
            "s",
            "s",
            "text, bigint, bytea, text, text, text, text, text",
            true,
        ),
        (
            "writer_lease_bind_runtime_v4",
            "f",
            "s",
            "s",
            "text, bigint, bytea, text, text, text, text, text",
            true,
        ),
        (
            "writer_lease_commit_plan_v1",
            "f",
            "v",
            "u",
            "text, bigint, bytea, bigint, bytea, text, bytea, text, text, bigint, bytea, bytea, bytea, bytea, bigint, bigint, bigint, bytea, text, bytea, text, text, text, bytea, text, text, text, text, bigint, bytea, text, bigint, bigint, text, bigint, text, bytea, bytea, bytea, bytea, bytea, text, text, bytea, bytea, bytea, text, bytea",
            true,
        ),
        ("writer_lease_load_commands_v1", "f", "s", "s", "text", true),
        ("writer_lease_load_current_v1", "f", "s", "s", "text", true),
        (
            "writer_lease_load_for_update_v1",
            "f",
            "v",
            "u",
            "text, bytea, bytea, bytea, text",
            true,
        ),
        (
            "writer_lease_load_for_update_v2",
            "f",
            "v",
            "u",
            "text, bytea, bytea, bytea, text",
            true,
        ),
        (
            "writer_lease_load_for_update_v3",
            "f",
            "v",
            "u",
            "text, bytea, bytea, bytea, text",
            true,
        ),
        (
            "writer_lease_load_for_update_v4",
            "f",
            "v",
            "u",
            "text, bytea, bytea, bytea, text",
            true,
        ),
        (
            "writer_lease_load_transitions_v1",
            "f",
            "s",
            "s",
            "text",
            true,
        ),
        ("writer_lease_rebind_v3", "p", "v", "u", "", false),
        ("writer_lease_rebind_v4", "p", "v", "u", "", false),
    ];
    if rows.len() != expected.len() {
        return Err(catalog_error());
    }
    for (row, (name, kind, volatility, parallel, args, security_definer)) in
        rows.iter().zip(expected)
    {
        if row_value::<String>(row, 0, PostgresStoreSetupErrorKind::CorruptCatalog)? != name
            || row_value::<String>(row, 1, PostgresStoreSetupErrorKind::CorruptCatalog)? != kind
            || row_value::<String>(row, 2, PostgresStoreSetupErrorKind::CorruptCatalog)?
                != "plpgsql"
            || row_value::<String>(row, 3, PostgresStoreSetupErrorKind::CorruptCatalog)?
                != "lattice_migrator"
            || row_value::<bool>(row, 4, PostgresStoreSetupErrorKind::CorruptCatalog)?
                != security_definer
            || row_value::<String>(row, 5, PostgresStoreSetupErrorKind::CorruptCatalog)?
                != volatility
            || row_value::<String>(row, 6, PostgresStoreSetupErrorKind::CorruptCatalog)? != parallel
            || row_value::<String>(row, 7, PostgresStoreSetupErrorKind::CorruptCatalog)? != args
            || row_value::<String>(row, 8, PostgresStoreSetupErrorKind::CorruptCatalog)?
                != "search_path=pg_catalog,row_security=on,lock_timeout=5s,statement_timeout=30s"
            || row_value::<bool>(row, 9, PostgresStoreSetupErrorKind::PermissionDenied)?
                != allowed(name)
        {
            return Err(catalog_error());
        }
        let (sql, delimiter, comment) = match name {
            "writer_lease_bind_runtime_v2" | "writer_lease_load_for_update_v2" => (
                WRITER_LEASE_V2_SQL,
                if name == "writer_lease_bind_runtime_v2" {
                    "lattice_writer_lease_bind_runtime_v2"
                } else {
                    "lattice_writer_lease_load_for_update_v2"
                },
                None,
            ),
            "writer_lease_bind_runtime_v3" | "writer_lease_load_for_update_v3" => (
                WRITER_LEASE_V3_SQL,
                if name == "writer_lease_bind_runtime_v3" {
                    "lattice_writer_lease_bind_runtime_v3"
                } else {
                    "lattice_writer_lease_load_for_update_v3"
                },
                if name == "writer_lease_bind_runtime_v3" {
                    Some("TASK087_GLOBAL_SCHEMA_V6_FOREMAN_COORDINATION_FOREMAN_SNAPSHOT_RECORDED")
                } else {
                    None
                },
            ),
            "writer_lease_bind_runtime_v4" | "writer_lease_load_for_update_v4" => (
                WRITER_LEASE_V4_SQL,
                if name == "writer_lease_bind_runtime_v4" {
                    "lattice_writer_lease_bind_runtime_v4"
                } else {
                    "lattice_writer_lease_load_for_update_v4"
                },
                if name == "writer_lease_bind_runtime_v4" {
                    Some("PHASE3_GLOBAL_SCHEMA_V7_GENERAL_TASK_INTAKE")
                } else {
                    None
                },
            ),
            "writer_lease_rebind_v3" => (
                WRITER_LEASE_V3_REBIND_SQL,
                "lattice_writer_lease_rebind_v3",
                Some("LATTICE_WRITER_LEASE_REBIND_V3"),
            ),
            "writer_lease_rebind_v4" => (
                WRITER_LEASE_V4_REBIND_SQL,
                "lattice_writer_lease_rebind_v4",
                Some("LATTICE_WRITER_LEASE_REBIND_V4"),
            ),
            _ => (
                WRITER_LEASE_V1_SQL,
                match name {
                    "writer_lease_assert_current_v1" => "lattice_writer_lease_assert_current_v1",
                    "writer_lease_bind_runtime_v1" => "lattice_writer_lease_bind_runtime_v1",
                    "writer_lease_commit_plan_v1" => "lattice_writer_lease_commit_plan_v1",
                    "writer_lease_load_commands_v1" => "lattice_writer_lease_load_commands_v1",
                    "writer_lease_load_current_v1" => "lattice_writer_lease_load_current_v1",
                    "writer_lease_load_for_update_v1" => "lattice_writer_lease_load_for_update_v1",
                    "writer_lease_load_transitions_v1" => {
                        "lattice_writer_lease_load_transitions_v1"
                    }
                    _ => return Err(catalog_error()),
                },
                None,
            ),
        };
        if row_value::<String>(row, 10, PostgresStoreSetupErrorKind::CorruptCatalog)?
            != embedded_writer_function_source(sql, delimiter)?
            || row_value::<String>(row, 11, PostgresStoreSetupErrorKind::CorruptCatalog)?
                != comment.unwrap_or("<NULL>")
        {
            return Err(catalog_error());
        }
    }
    let closure = client
        .query_one(
            "SELECT \
             (SELECT count(*) FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace \
               WHERE n.nspname='writer_lease' AND pg_catalog.has_table_privilege(
                 'lattice_runtime',c.oid,'SELECT,INSERT,UPDATE,DELETE,TRUNCATE,REFERENCES,TRIGGER,MAINTAIN')), \
             (SELECT count(*) FROM pg_catalog.pg_proc p JOIN pg_catalog.pg_namespace n ON n.oid=p.pronamespace \
               CROSS JOIN pg_catalog.pg_roles r WHERE n.nspname='writer_lease' AND NOT r.rolsuper \
                 AND r.rolname !~ '^pg_' AND r.rolname NOT IN ('lattice_migrator','lattice_runtime') \
                 AND pg_catalog.has_function_privilege(r.rolname,p.oid,'EXECUTE')), \
             pg_catalog.has_schema_privilege('lattice_runtime','writer_lease','CREATE')",
            &[],
        )
        .map_err(|error| map_postgres_error(&error, PostgresStoreSetupErrorKind::PermissionDenied))?;
    if row_value::<i64>(&closure, 0, PostgresStoreSetupErrorKind::PermissionDenied)? != 0
        || row_value::<i64>(&closure, 1, PostgresStoreSetupErrorKind::PermissionDenied)? != 0
        || row_value::<bool>(&closure, 2, PostgresStoreSetupErrorKind::PermissionDenied)?
    {
        return Err(permission_error());
    }
    Ok(())
}

#[allow(clippy::too_many_lines)]
fn verify_writer_lease_v5_functions<C: GenericClient>(
    client: &mut C,
    runtime_profile: WriterLeaseV5RuntimeProfile,
) -> Result<(), PostgresStoreSetupError> {
    if WRITER_LEASE_V5_SQL.len() != 20_740
        || hex_digest(Sha256::digest(WRITER_LEASE_V5_SQL.as_bytes()).as_ref())
            != WRITER_LEASE_V5_SQL_SHA256
        || WRITER_LEASE_V5_STORE_V8_REBIND_SQL.len() != 14_932
        || hex_digest(Sha256::digest(WRITER_LEASE_V5_STORE_V8_REBIND_SQL.as_bytes()).as_ref())
            != WRITER_LEASE_V5_STORE_V8_REBIND_SQL_SHA256
        || WRITER_LEASE_V4_REBIND_SQL.is_empty()
    {
        return Err(catalog_error());
    }
    verify_writer_lease_v5_transition_constraint(client)?;
    let rows = client
        .query(
            "SELECT p.proname::text,p.prokind::text,l.lanname,r.rolname,p.prosecdef, \
                    p.provolatile::text,p.proparallel::text, \
                    pg_catalog.oidvectortypes(p.proargtypes), \
                    COALESCE(pg_catalog.array_to_string(p.proconfig,','),'<NULL>'), \
                    pg_catalog.has_function_privilege('lattice_runtime',p.oid,'EXECUTE'), \
                    p.prosrc::text,COALESCE(pg_catalog.obj_description(p.oid,'pg_proc'),'<NULL>') \
               FROM pg_catalog.pg_proc p \
               JOIN pg_catalog.pg_namespace n ON n.oid=p.pronamespace \
               JOIN pg_catalog.pg_language l ON l.oid=p.prolang \
               JOIN pg_catalog.pg_roles r ON r.oid=p.proowner \
              WHERE n.nspname='writer_lease' \
              ORDER BY p.proname,pg_catalog.pg_get_function_identity_arguments(p.oid)",
            &[],
        )
        .map_err(|error| map_postgres_error(&error, PostgresStoreSetupErrorKind::CorruptCatalog))?;
    let allowed = |name: &str| {
        matches!(
            name,
            "writer_lease_assert_current_v1"
                | "writer_lease_bind_runtime_v5"
                | "writer_lease_commit_plan_v1"
                | "writer_lease_load_commands_v1"
                | "writer_lease_load_current_v1"
                | "writer_lease_load_for_update_v5"
                | "writer_lease_load_transitions_v1"
        )
    };
    let expected = [
        (
            "writer_lease_assert_current_v1",
            "f",
            "s",
            "s",
            "text, text, text, text, bytea, text, text, text, text, bigint, bytea, text, bigint, bigint, bytea",
            true,
        ),
        (
            "writer_lease_bind_runtime_v1",
            "f",
            "s",
            "s",
            "text, bigint, bytea, text, text, text, text, text",
            true,
        ),
        (
            "writer_lease_bind_runtime_v2",
            "f",
            "s",
            "s",
            "text, bigint, bytea, text, text, text, text, text",
            true,
        ),
        (
            "writer_lease_bind_runtime_v3",
            "f",
            "s",
            "s",
            "text, bigint, bytea, text, text, text, text, text",
            true,
        ),
        (
            "writer_lease_bind_runtime_v4",
            "f",
            "s",
            "s",
            "text, bigint, bytea, text, text, text, text, text",
            true,
        ),
        (
            "writer_lease_bind_runtime_v5",
            "f",
            "s",
            "s",
            "text, bigint, bytea, text, text, text, text, text",
            true,
        ),
        (
            "writer_lease_commit_plan_v1",
            "f",
            "v",
            "u",
            "text, bigint, bytea, bigint, bytea, text, bytea, text, text, bigint, bytea, bytea, bytea, bytea, bigint, bigint, bigint, bytea, text, bytea, text, text, text, bytea, text, text, text, text, bigint, bytea, text, bigint, bigint, text, bigint, text, bytea, bytea, bytea, bytea, bytea, text, text, bytea, bytea, bytea, text, bytea",
            true,
        ),
        ("writer_lease_load_commands_v1", "f", "s", "s", "text", true),
        ("writer_lease_load_current_v1", "f", "s", "s", "text", true),
        (
            "writer_lease_load_for_update_v1",
            "f",
            "v",
            "u",
            "text, bytea, bytea, bytea, text",
            true,
        ),
        (
            "writer_lease_load_for_update_v2",
            "f",
            "v",
            "u",
            "text, bytea, bytea, bytea, text",
            true,
        ),
        (
            "writer_lease_load_for_update_v3",
            "f",
            "v",
            "u",
            "text, bytea, bytea, bytea, text",
            true,
        ),
        (
            "writer_lease_load_for_update_v4",
            "f",
            "v",
            "u",
            "text, bytea, bytea, bytea, text",
            true,
        ),
        (
            "writer_lease_load_for_update_v5",
            "f",
            "v",
            "u",
            "text, bytea, bytea, bytea, text",
            true,
        ),
        (
            "writer_lease_load_transitions_v1",
            "f",
            "s",
            "s",
            "text",
            true,
        ),
        ("writer_lease_rebind_v3", "p", "v", "u", "", false),
        ("writer_lease_rebind_v4", "p", "v", "u", "", false),
    ];
    if rows.len() != expected.len() {
        return Err(catalog_error());
    }
    for (row, (name, kind, volatility, parallel, args, security_definer)) in
        rows.iter().zip(expected)
    {
        if row_value::<String>(row, 0, PostgresStoreSetupErrorKind::CorruptCatalog)? != name
            || row_value::<String>(row, 1, PostgresStoreSetupErrorKind::CorruptCatalog)? != kind
            || row_value::<String>(row, 2, PostgresStoreSetupErrorKind::CorruptCatalog)?
                != "plpgsql"
            || row_value::<String>(row, 3, PostgresStoreSetupErrorKind::CorruptCatalog)?
                != "lattice_migrator"
            || row_value::<bool>(row, 4, PostgresStoreSetupErrorKind::CorruptCatalog)?
                != security_definer
            || row_value::<String>(row, 5, PostgresStoreSetupErrorKind::CorruptCatalog)?
                != volatility
            || row_value::<String>(row, 6, PostgresStoreSetupErrorKind::CorruptCatalog)? != parallel
            || row_value::<String>(row, 7, PostgresStoreSetupErrorKind::CorruptCatalog)? != args
            || row_value::<String>(row, 8, PostgresStoreSetupErrorKind::CorruptCatalog)?
                != "search_path=pg_catalog,row_security=on,lock_timeout=5s,statement_timeout=30s"
            || row_value::<bool>(row, 9, PostgresStoreSetupErrorKind::PermissionDenied)?
                != allowed(name)
        {
            return Err(catalog_error());
        }
        let (sql, delimiter, comment) = match name {
            "writer_lease_bind_runtime_v2" | "writer_lease_load_for_update_v2" => (
                WRITER_LEASE_V2_SQL,
                if name == "writer_lease_bind_runtime_v2" {
                    "lattice_writer_lease_bind_runtime_v2"
                } else {
                    "lattice_writer_lease_load_for_update_v2"
                },
                None,
            ),
            "writer_lease_bind_runtime_v3" | "writer_lease_load_for_update_v3" => (
                WRITER_LEASE_V3_SQL,
                if name == "writer_lease_bind_runtime_v3" {
                    "lattice_writer_lease_bind_runtime_v3"
                } else {
                    "lattice_writer_lease_load_for_update_v3"
                },
                if name == "writer_lease_bind_runtime_v3" {
                    Some("TASK087_GLOBAL_SCHEMA_V6_FOREMAN_COORDINATION_FOREMAN_SNAPSHOT_RECORDED")
                } else {
                    None
                },
            ),
            "writer_lease_bind_runtime_v4" | "writer_lease_load_for_update_v4" => (
                WRITER_LEASE_V4_SQL,
                if name == "writer_lease_bind_runtime_v4" {
                    "lattice_writer_lease_bind_runtime_v4"
                } else {
                    "lattice_writer_lease_load_for_update_v4"
                },
                if name == "writer_lease_bind_runtime_v4" {
                    Some("PHASE3_GLOBAL_SCHEMA_V7_GENERAL_TASK_INTAKE")
                } else {
                    None
                },
            ),
            "writer_lease_bind_runtime_v5" | "writer_lease_load_for_update_v5" => (
                match runtime_profile {
                    WriterLeaseV5RuntimeProfile::StoreV7Base => WRITER_LEASE_V5_SQL,
                    WriterLeaseV5RuntimeProfile::StoreV8Successor => {
                        WRITER_LEASE_V5_STORE_V8_REBIND_SQL
                    }
                },
                if name == "writer_lease_bind_runtime_v5" {
                    "lattice_writer_lease_bind_runtime_v5"
                } else {
                    "lattice_writer_lease_load_for_update_v5"
                },
                Some("PHASE4_EXACT_PROCESS_HANDOFF"),
            ),
            "writer_lease_rebind_v3" => (
                WRITER_LEASE_V3_REBIND_SQL,
                "lattice_writer_lease_rebind_v3",
                Some("LATTICE_WRITER_LEASE_REBIND_V3"),
            ),
            "writer_lease_rebind_v4" => (
                WRITER_LEASE_V4_REBIND_SQL,
                "lattice_writer_lease_rebind_v4",
                Some("LATTICE_WRITER_LEASE_REBIND_V4"),
            ),
            _ => (
                WRITER_LEASE_V1_SQL,
                match name {
                    "writer_lease_assert_current_v1" => "lattice_writer_lease_assert_current_v1",
                    "writer_lease_bind_runtime_v1" => "lattice_writer_lease_bind_runtime_v1",
                    "writer_lease_commit_plan_v1" => "lattice_writer_lease_commit_plan_v1",
                    "writer_lease_load_commands_v1" => "lattice_writer_lease_load_commands_v1",
                    "writer_lease_load_current_v1" => "lattice_writer_lease_load_current_v1",
                    "writer_lease_load_for_update_v1" => "lattice_writer_lease_load_for_update_v1",
                    "writer_lease_load_transitions_v1" => {
                        "lattice_writer_lease_load_transitions_v1"
                    }
                    _ => return Err(catalog_error()),
                },
                None,
            ),
        };
        if row_value::<String>(row, 10, PostgresStoreSetupErrorKind::CorruptCatalog)?
            != embedded_writer_function_source(sql, delimiter)?
            || row_value::<String>(row, 11, PostgresStoreSetupErrorKind::CorruptCatalog)?
                != comment.unwrap_or("<NULL>")
        {
            return Err(catalog_error());
        }
    }
    let closure = client
        .query_one(
            "SELECT \
             (SELECT count(*) FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace \
               WHERE n.nspname='writer_lease' AND pg_catalog.has_table_privilege(
                 'lattice_runtime',c.oid,'SELECT,INSERT,UPDATE,DELETE,TRUNCATE,REFERENCES,TRIGGER,MAINTAIN')), \
             (SELECT count(*) FROM pg_catalog.pg_proc p JOIN pg_catalog.pg_namespace n ON n.oid=p.pronamespace \
               CROSS JOIN pg_catalog.pg_roles r WHERE n.nspname='writer_lease' AND NOT r.rolsuper \
                 AND r.rolname !~ '^pg_' AND r.rolname NOT IN ('lattice_migrator','lattice_runtime') \
                 AND pg_catalog.has_function_privilege(r.rolname,p.oid,'EXECUTE')), \
             pg_catalog.has_schema_privilege('lattice_runtime','writer_lease','CREATE')",
            &[],
        )
        .map_err(|error| map_postgres_error(&error, PostgresStoreSetupErrorKind::PermissionDenied))?;
    if row_value::<i64>(&closure, 0, PostgresStoreSetupErrorKind::PermissionDenied)? != 0
        || row_value::<i64>(&closure, 1, PostgresStoreSetupErrorKind::PermissionDenied)? != 0
        || row_value::<bool>(&closure, 2, PostgresStoreSetupErrorKind::PermissionDenied)?
    {
        return Err(permission_error());
    }
    Ok(())
}

fn verify_writer_lease_v5_transition_constraint<C: GenericClient>(
    client: &mut C,
) -> Result<(), PostgresStoreSetupError> {
    let row = client
        .query_one(
            "SELECT pg_catalog.count(*), \
                    pg_catalog.max(pg_catalog.pg_get_constraintdef(c.oid,false)) \
               FROM pg_catalog.pg_constraint c \
               JOIN pg_catalog.pg_namespace n ON n.oid=c.connamespace \
              WHERE n.nspname='writer_lease' \
                AND c.conname IN ('writer_lease_transitions_identity', \
                                  'writer_lease_transitions_identity_v5')",
            &[],
        )
        .map_err(|error| map_postgres_error(&error, PostgresStoreSetupErrorKind::CorruptCatalog))?;
    let definition =
        row_value::<Option<String>>(&row, 1, PostgresStoreSetupErrorKind::CorruptCatalog)?;
    if row_value::<i64>(&row, 0, PostgresStoreSetupErrorKind::CorruptCatalog)? != 1
        || definition.as_deref().is_none_or(|definition| {
            !definition.contains("transition_kind")
                || !definition.contains("PROCESS_HANDOFF")
                || !definition.contains("MARK_SUSPECT")
                || !definition.contains("REVOKE")
        })
    {
        return Err(catalog_error());
    }
    Ok(())
}

fn verify_writer_lease_v2_acl_closure<C: GenericClient>(
    client: &mut C,
    runtime: WriterLeaseV2RuntimeProfile,
) -> Result<(), PostgresStoreSetupError> {
    let expected_missing = match runtime {
        WriterLeaseV2RuntimeProfile::Bridge => 9_i64,
        WriterLeaseV2RuntimeProfile::Current => 2_i64,
    };
    let expected_usage = runtime == WriterLeaseV2RuntimeProfile::Current;
    verify_writer_lease_acl_closure(client, expected_missing, expected_usage)
}

const WRITER_LEASE_ACL_CLOSURE_SQL: &str = "SELECT \
             (SELECT count(*) FROM pg_catalog.pg_trigger tr \
               JOIN pg_catalog.pg_class c ON c.oid=tr.tgrelid \
               JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace \
              WHERE n.nspname='writer_lease' AND tr.tgisinternal \
                AND tr.tgenabled='O' AND tr.tgconstraint<>0), \
             (SELECT count(*) FROM pg_catalog.pg_trigger tr \
               JOIN pg_catalog.pg_class c ON c.oid=tr.tgrelid \
               JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace \
              WHERE n.nspname='writer_lease' AND NOT tr.tgisinternal), \
             (SELECT count(*) FROM pg_catalog.pg_rewrite rw \
               JOIN pg_catalog.pg_class c ON c.oid=rw.ev_class \
               JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace \
              WHERE n.nspname='writer_lease'), \
             (SELECT count(*) FROM pg_catalog.pg_policy p \
               JOIN pg_catalog.pg_class c ON c.oid=p.polrelid \
               JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace \
              WHERE n.nspname='writer_lease'), \
             (SELECT count(*) FROM pg_catalog.pg_inherits i \
               JOIN pg_catalog.pg_class c ON c.oid=i.inhrelid \
               JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace \
              WHERE n.nspname='writer_lease'), \
             (SELECT count(*) FROM pg_catalog.pg_default_acl d \
               JOIN pg_catalog.pg_namespace n ON n.oid=d.defaclnamespace \
              WHERE n.nspname='writer_lease'), \
             (SELECT count(*) FROM pg_catalog.pg_class c \
               JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace \
              WHERE n.nspname='writer_lease' AND c.relkind NOT IN ('r','i')), \
             ((SELECT count(*) FROM pg_catalog.pg_collation x \
                JOIN pg_catalog.pg_namespace n ON n.oid=x.collnamespace WHERE n.nspname='writer_lease') + \
              (SELECT count(*) FROM pg_catalog.pg_conversion x \
                JOIN pg_catalog.pg_namespace n ON n.oid=x.connamespace WHERE n.nspname='writer_lease') + \
              (SELECT count(*) FROM pg_catalog.pg_operator x \
                JOIN pg_catalog.pg_namespace n ON n.oid=x.oprnamespace WHERE n.nspname='writer_lease') + \
              (SELECT count(*) FROM pg_catalog.pg_opclass x \
                JOIN pg_catalog.pg_namespace n ON n.oid=x.opcnamespace WHERE n.nspname='writer_lease') + \
              (SELECT count(*) FROM pg_catalog.pg_opfamily x \
                JOIN pg_catalog.pg_namespace n ON n.oid=x.opfnamespace WHERE n.nspname='writer_lease') + \
              (SELECT count(*) FROM pg_catalog.pg_statistic_ext x \
                JOIN pg_catalog.pg_namespace n ON n.oid=x.stxnamespace WHERE n.nspname='writer_lease') + \
              (SELECT count(*) FROM pg_catalog.pg_ts_config x \
                JOIN pg_catalog.pg_namespace n ON n.oid=x.cfgnamespace WHERE n.nspname='writer_lease') + \
              (SELECT count(*) FROM pg_catalog.pg_ts_dict x \
                JOIN pg_catalog.pg_namespace n ON n.oid=x.dictnamespace WHERE n.nspname='writer_lease') + \
              (SELECT count(*) FROM pg_catalog.pg_ts_parser x \
                JOIN pg_catalog.pg_namespace n ON n.oid=x.prsnamespace WHERE n.nspname='writer_lease') + \
              (SELECT count(*) FROM pg_catalog.pg_ts_template x \
                JOIN pg_catalog.pg_namespace n ON n.oid=x.tmplnamespace WHERE n.nspname='writer_lease') + \
              (SELECT count(*) FROM pg_catalog.pg_cast c \
                WHERE c.castsource IN (SELECT t.oid FROM pg_catalog.pg_type t \
                      JOIN pg_catalog.pg_namespace n ON n.oid=t.typnamespace \
                     WHERE n.nspname='writer_lease') \
                   OR c.casttarget IN (SELECT t.oid FROM pg_catalog.pg_type t \
                      JOIN pg_catalog.pg_namespace n ON n.oid=t.typnamespace \
                     WHERE n.nspname='writer_lease') \
                    OR c.castfunc IN (SELECT p.oid FROM pg_catalog.pg_proc p \
                       JOIN pg_catalog.pg_namespace n ON n.oid=p.pronamespace \
                      WHERE n.nspname='writer_lease')) + \
              (SELECT count(*) FROM pg_catalog.pg_transform tr \
                WHERE tr.trftype IN (SELECT t.oid FROM pg_catalog.pg_type t \
                      JOIN pg_catalog.pg_namespace n ON n.oid=t.typnamespace \
                     WHERE n.nspname='writer_lease') \
                   OR tr.trffromsql IN (SELECT p.oid FROM pg_catalog.pg_proc p \
                      JOIN pg_catalog.pg_namespace n ON n.oid=p.pronamespace \
                     WHERE n.nspname='writer_lease') \
                   OR tr.trftosql IN (SELECT p.oid FROM pg_catalog.pg_proc p \
                      JOIN pg_catalog.pg_namespace n ON n.oid=p.pronamespace \
                     WHERE n.nspname='writer_lease'))), \
             (SELECT count(*) FROM pg_catalog.pg_proc p \
               JOIN pg_catalog.pg_namespace n ON n.oid=p.pronamespace \
               CROSS JOIN pg_catalog.pg_roles roles \
              WHERE n.nspname='writer_lease' AND NOT roles.rolsuper \
                AND roles.rolname !~ '^pg_' \
                AND roles.rolname NOT IN ('lattice_migrator','lattice_runtime') \
                AND pg_catalog.has_function_privilege(roles.rolname,p.oid,'EXECUTE')), \
             (SELECT count(*) FROM pg_catalog.pg_class c \
               JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace \
               CROSS JOIN pg_catalog.pg_roles roles \
              WHERE n.nspname='writer_lease' AND c.relkind IN ('r','p') \
                AND NOT roles.rolsuper AND roles.rolname !~ '^pg_' \
                AND roles.rolname <> 'lattice_migrator' \
                AND pg_catalog.has_table_privilege(roles.rolname,c.oid, \
                    'SELECT,INSERT,UPDATE,DELETE,TRUNCATE,REFERENCES,TRIGGER,MAINTAIN')), \
             (SELECT count(*) FROM pg_catalog.pg_proc p \
               JOIN pg_catalog.pg_namespace n ON n.oid=p.pronamespace \
              WHERE n.nspname='writer_lease' \
                AND NOT pg_catalog.has_function_privilege('lattice_runtime',p.oid,'EXECUTE')), \
             (SELECT count(*) FROM pg_catalog.pg_roles roles \
              WHERE NOT roles.rolsuper AND roles.rolname !~ '^pg_' \
                AND roles.rolname NOT IN ('lattice_migrator','lattice_runtime') \
                AND (pg_catalog.has_schema_privilege(roles.rolname,'writer_lease','USAGE') \
                  OR pg_catalog.has_schema_privilege(roles.rolname,'writer_lease','CREATE'))), \
             (SELECT count(*) FROM ( \
                SELECT t.oid \
                  FROM pg_catalog.pg_type t \
                  JOIN pg_catalog.pg_namespace n ON n.oid=t.typnamespace \
                  LEFT JOIN LATERAL pg_catalog.aclexplode( \
                    COALESCE(t.typacl,pg_catalog.acldefault('T',t.typowner))) acl ON TRUE \
                 WHERE n.nspname='writer_lease' \
                 GROUP BY t.oid,t.typowner \
                HAVING count(acl.privilege_type)<>2 \
                    OR count(*) FILTER (WHERE acl.grantee=0 \
                        AND acl.grantor=t.typowner \
                        AND acl.privilege_type='USAGE' AND NOT acl.is_grantable)<>1 \
                    OR count(*) FILTER (WHERE acl.grantee=t.typowner \
                        AND acl.grantor=t.typowner \
                        AND acl.privilege_type='USAGE' AND NOT acl.is_grantable)<>1 \
             ) type_acl_drift)";

fn verify_writer_lease_acl_closure<C: GenericClient>(
    client: &mut C,
    expected_missing: i64,
    expected_usage: bool,
) -> Result<(), PostgresStoreSetupError> {
    let closure = client
        .query_one(WRITER_LEASE_ACL_CLOSURE_SQL, &[])
        .map_err(|error| {
            map_postgres_error(&error, PostgresStoreSetupErrorKind::PermissionDenied)
        })?;
    verify_writer_lease_acl_closure_counts(&closure, expected_missing)?;
    let usage = client
        .query_one(
            "SELECT pg_catalog.has_schema_privilege('lattice_runtime','writer_lease','USAGE')",
            &[],
        )
        .map_err(|error| {
            map_postgres_error(&error, PostgresStoreSetupErrorKind::PermissionDenied)
        })?;
    if row_value::<bool>(&usage, 0, PostgresStoreSetupErrorKind::PermissionDenied)?
        != expected_usage
    {
        return Err(permission_error());
    }
    Ok(())
}

fn verify_writer_lease_acl_closure_counts(
    closure: &Row,
    expected_missing: i64,
) -> Result<(), PostgresStoreSetupError> {
    for (index, expected) in [(0, 12_i64), (1, 0), (2, 0), (3, 0), (4, 0), (6, 0), (7, 0)] {
        if row_value::<i64>(closure, index, PostgresStoreSetupErrorKind::CorruptCatalog)?
            != expected
        {
            return Err(catalog_error());
        }
    }
    for (index, expected) in [
        (5, 0_i64),
        (8, 0),
        (9, 0),
        (10, expected_missing),
        (11, 0),
        (12, 0),
    ] {
        if row_value::<i64>(
            closure,
            index,
            PostgresStoreSetupErrorKind::PermissionDenied,
        )? != expected
        {
            return Err(permission_error());
        }
    }
    Ok(())
}

fn verify_catalog_signatures<C: GenericClient>(
    client: &mut C,
    profile: CatalogProfile,
    v3_prefix: bool,
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
            if v3_prefix {
                V3_PREFIX_EXPECTED_CONSTRAINT_SIGNATURE
            } else {
                V3_EXPECTED_CONSTRAINT_SIGNATURE
            },
            V3_EXPECTED_INDEX_SIGNATURE,
        ],
        CatalogProfile::V3CodebaseMemoryV2
        | CatalogProfile::V3CodebaseMemoryV2WriterLeaseV1
        | CatalogProfile::V3CodebaseMemoryV2WriterLeaseV2Bridge => [
            V3_CODEBASE_MEMORY_V2_EXPECTED_RELATION_SIGNATURE,
            V3_CODEBASE_MEMORY_V2_EXPECTED_COLUMN_SIGNATURE,
            if v3_prefix {
                V3_CODEBASE_MEMORY_V2_PREFIX_EXPECTED_CONSTRAINT_SIGNATURE
            } else {
                V3_CODEBASE_MEMORY_V2_EXPECTED_CONSTRAINT_SIGNATURE
            },
            V3_CODEBASE_MEMORY_V2_EXPECTED_INDEX_SIGNATURE,
        ],
        CatalogProfile::V4 => [
            V4_EXPECTED_RELATION_SIGNATURE,
            V4_EXPECTED_COLUMN_SIGNATURE,
            V4_EXPECTED_CONSTRAINT_SIGNATURE,
            V4_EXPECTED_INDEX_SIGNATURE,
        ],
        CatalogProfile::V5 => [
            V4_EXPECTED_RELATION_SIGNATURE,
            V5_EXPECTED_COLUMN_SIGNATURE,
            V5_EXPECTED_CONSTRAINT_SIGNATURE,
            V4_EXPECTED_INDEX_SIGNATURE,
        ],
        CatalogProfile::V5CodebaseMemoryV2UpgradePending
        | CatalogProfile::V5CodebaseMemoryV2WriterLeaseV2BridgePending => [
            V5_CODEBASE_MEMORY_V2_EXPECTED_RELATION_SIGNATURE,
            V5_CODEBASE_MEMORY_V2_EXPECTED_COLUMN_SIGNATURE,
            V5_CODEBASE_MEMORY_V2_EXPECTED_CONSTRAINT_SIGNATURE,
            V5_CODEBASE_MEMORY_V2_EXPECTED_INDEX_SIGNATURE,
        ],
        CatalogProfile::V5CodebaseMemoryV3Current
        | CatalogProfile::V5CodebaseMemoryV3WriterLeaseV2BridgePending
        | CatalogProfile::V5CodebaseMemoryV3WriterLeaseV2Current => [
            V5_CODEBASE_MEMORY_V3_EXPECTED_RELATION_SIGNATURE,
            V5_CODEBASE_MEMORY_V3_EXPECTED_COLUMN_SIGNATURE,
            V5_CODEBASE_MEMORY_V3_EXPECTED_CONSTRAINT_SIGNATURE,
            V5_CODEBASE_MEMORY_V3_EXPECTED_INDEX_SIGNATURE,
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
    let suffix = match profile {
        CatalogProfile::V1 => "V1",
        CatalogProfile::V2 => "V2",
        CatalogProfile::V3
        | CatalogProfile::V3CodebaseMemoryV2
        | CatalogProfile::V3CodebaseMemoryV2WriterLeaseV1
        | CatalogProfile::V3CodebaseMemoryV2WriterLeaseV2Bridge => "V3",
        CatalogProfile::V4 => "V4",
        CatalogProfile::V5
        | CatalogProfile::V5CodebaseMemoryV2UpgradePending
        | CatalogProfile::V5CodebaseMemoryV3Current
        | CatalogProfile::V5CodebaseMemoryV2WriterLeaseV2BridgePending
        | CatalogProfile::V5CodebaseMemoryV3WriterLeaseV2BridgePending
        | CatalogProfile::V5CodebaseMemoryV3WriterLeaseV2Current => "V5",
        CatalogProfile::PreSchema => return Err(catalog_error()),
    };
    verify_schema_header_comments(client, suffix)?;
    let tables = string_set(
        client,
        "SELECT c.relname FROM pg_class c \
         JOIN pg_namespace n ON n.oid = c.relnamespace \
         WHERE n.nspname = 'control' AND c.relkind = 'r' \
           AND c.relname <> 'task_ledger_autonomy_receipts' ORDER BY c.relname",
    )?;
    let expected_tables: BTreeSet<String> = match profile {
        CatalogProfile::V1 | CatalogProfile::V2 => {
            CONTROL_TABLES.into_iter().map(str::to_owned).collect()
        }
        CatalogProfile::V3
        | CatalogProfile::V3CodebaseMemoryV2
        | CatalogProfile::V3CodebaseMemoryV2WriterLeaseV1
        | CatalogProfile::V3CodebaseMemoryV2WriterLeaseV2Bridge => {
            V3_CONTROL_TABLES.into_iter().map(str::to_owned).collect()
        }
        CatalogProfile::V4
        | CatalogProfile::V5
        | CatalogProfile::V5CodebaseMemoryV2UpgradePending
        | CatalogProfile::V5CodebaseMemoryV3Current
        | CatalogProfile::V5CodebaseMemoryV2WriterLeaseV2BridgePending
        | CatalogProfile::V5CodebaseMemoryV3WriterLeaseV2BridgePending
        | CatalogProfile::V5CodebaseMemoryV3WriterLeaseV2Current => {
            V4_CONTROL_TABLES.into_iter().map(str::to_owned).collect()
        }
        CatalogProfile::PreSchema => return Err(catalog_error()),
    };
    if tables != expected_tables {
        return Err(catalog_error());
    }
    let constraints = string_set(
        client,
        "SELECT con.conname FROM pg_constraint con \
         JOIN pg_namespace n ON n.oid = con.connamespace \
         JOIN pg_class c ON c.oid = con.conrelid \
         WHERE n.nspname = 'control' \
           AND c.relname <> 'task_ledger_autonomy_receipts' ORDER BY con.conname",
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
        | CatalogProfile::V3CodebaseMemoryV2WriterLeaseV1
        | CatalogProfile::V3CodebaseMemoryV2WriterLeaseV2Bridge => V2_CONTROL_CONSTRAINTS
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
        CatalogProfile::V5
        | CatalogProfile::V5CodebaseMemoryV2UpgradePending
        | CatalogProfile::V5CodebaseMemoryV3Current
        | CatalogProfile::V5CodebaseMemoryV2WriterLeaseV2BridgePending
        | CatalogProfile::V5CodebaseMemoryV3WriterLeaseV2BridgePending
        | CatalogProfile::V5CodebaseMemoryV3WriterLeaseV2Current => V2_CONTROL_CONSTRAINTS
            .into_iter()
            .chain(TASK_LEDGER_CONTROL_CONSTRAINTS)
            .chain(PROJECT_REGISTRY_CONTROL_CONSTRAINTS)
            .chain(["project_registry_commands_persistence_profile"])
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

fn verify_schema_header_comments<C: GenericClient>(
    client: &mut C,
    suffix: &str,
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
        CatalogProfile::V3CodebaseMemoryV2WriterLeaseV1
        | CatalogProfile::V3CodebaseMemoryV2WriterLeaseV2Bridge => V3_CONTROL_TABLES
            .into_iter()
            .map(|table| ("control", table))
            .chain(
                CODEBASE_MEMORY_V2_TABLES
                    .into_iter()
                    .map(|table| ("memory", table)),
            )
            .collect(),
        CatalogProfile::V4 | CatalogProfile::V5 => V4_CONTROL_TABLES
            .into_iter()
            .map(|table| ("control", table))
            .collect(),
        CatalogProfile::V5CodebaseMemoryV2UpgradePending
        | CatalogProfile::V5CodebaseMemoryV3Current
        | CatalogProfile::V5CodebaseMemoryV2WriterLeaseV2BridgePending
        | CatalogProfile::V5CodebaseMemoryV3WriterLeaseV2BridgePending
        | CatalogProfile::V5CodebaseMemoryV3WriterLeaseV2Current => V4_CONTROL_TABLES
            .into_iter()
            .map(|table| ("control", table))
            .chain(
                CODEBASE_MEMORY_V2_TABLES
                    .into_iter()
                    .map(|table| ("memory", table)),
            )
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

const FORBIDDEN_SCHEMA_OBJECTS_SQL: &str = "SELECT \
             (SELECT count(*) FROM pg_proc p JOIN pg_namespace n ON n.oid = p.pronamespace \
              WHERE n.nspname IN ('control', 'memory', 'readmodel') \
                AND p.proname NOT IN ('task_ledger_record_autonomy_receipt_v1', \
                                      'task_ledger_read_autonomy_receipts_v1')), \
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
                 AND c.relname <> 'task_ledger_autonomy_receipts' \
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
                 AND (c.relhassubclass OR c.relispartition))";

fn read_forbidden_schema_object_counts<C: GenericClient>(
    client: &mut C,
) -> Result<[i64; 10], PostgresStoreSetupError> {
    let row = client
        .query_one(FORBIDDEN_SCHEMA_OBJECTS_SQL, &[])
        .map_err(|error| map_postgres_error(&error, PostgresStoreSetupErrorKind::CorruptCatalog))?;
    let mut counts = [0_i64; 10];
    for (index, count) in counts.iter_mut().enumerate() {
        *count = row_value::<i64>(&row, index, PostgresStoreSetupErrorKind::CorruptCatalog)?;
    }
    Ok(counts)
}

#[allow(clippy::too_many_lines)]
fn verify_forbidden_schema_objects<C: GenericClient>(
    client: &mut C,
    profile: CatalogProfile,
    v3_prefix: bool,
) -> Result<(), PostgresStoreSetupError> {
    let forbidden = read_forbidden_schema_object_counts(client)?;
    let expected_functions = expected_owned_function_count(profile);
    if forbidden[0] != expected_functions {
        return Err(catalog_error());
    }
    if forbidden[1..6].iter().any(|count| *count != 0) {
        return Err(catalog_error());
    }
    let expected_scope_head_triggers = expected_scope_head_trigger_count(profile);
    let expected_internal_triggers = expected_internal_trigger_count(profile, v3_prefix);
    if forbidden[6] != expected_scope_head_triggers
        || forbidden[7] != expected_internal_triggers
        || forbidden[8] != 0
        || forbidden[9] != 0
    {
        return Err(catalog_error());
    }
    Ok(())
}

fn expected_owned_function_count(profile: CatalogProfile) -> i64 {
    match profile {
        CatalogProfile::V1 | CatalogProfile::PreSchema => 0,
        CatalogProfile::V2 => 3,
        CatalogProfile::V3 => 11,
        CatalogProfile::V3CodebaseMemoryV2
        | CatalogProfile::V3CodebaseMemoryV2WriterLeaseV1
        | CatalogProfile::V3CodebaseMemoryV2WriterLeaseV2Bridge => 18,
        CatalogProfile::V4 => 28,
        CatalogProfile::V5 => 45,
        CatalogProfile::V5CodebaseMemoryV2UpgradePending
        | CatalogProfile::V5CodebaseMemoryV2WriterLeaseV2BridgePending => 52,
        CatalogProfile::V5CodebaseMemoryV3Current
        | CatalogProfile::V5CodebaseMemoryV3WriterLeaseV2BridgePending
        | CatalogProfile::V5CodebaseMemoryV3WriterLeaseV2Current => 59,
    }
}

fn expected_scope_head_trigger_count(profile: CatalogProfile) -> i64 {
    match profile {
        CatalogProfile::V1 => 4,
        CatalogProfile::V2
        | CatalogProfile::V3
        | CatalogProfile::V3CodebaseMemoryV2
        | CatalogProfile::V3CodebaseMemoryV2WriterLeaseV1
        | CatalogProfile::V3CodebaseMemoryV2WriterLeaseV2Bridge
        | CatalogProfile::V4
        | CatalogProfile::V5
        | CatalogProfile::V5CodebaseMemoryV2UpgradePending
        | CatalogProfile::V5CodebaseMemoryV3Current
        | CatalogProfile::V5CodebaseMemoryV2WriterLeaseV2BridgePending
        | CatalogProfile::V5CodebaseMemoryV3WriterLeaseV2BridgePending
        | CatalogProfile::V5CodebaseMemoryV3WriterLeaseV2Current
        | CatalogProfile::PreSchema => 0,
    }
}

fn expected_internal_trigger_count(profile: CatalogProfile, v3_prefix: bool) -> i64 {
    match profile {
        CatalogProfile::V1 => 4,
        CatalogProfile::V2 | CatalogProfile::PreSchema => 0,
        CatalogProfile::V3 if v3_prefix => 20,
        CatalogProfile::V3 => 22,
        CatalogProfile::V3CodebaseMemoryV2
        | CatalogProfile::V3CodebaseMemoryV2WriterLeaseV1
        | CatalogProfile::V3CodebaseMemoryV2WriterLeaseV2Bridge
            if v3_prefix =>
        {
            44
        }
        CatalogProfile::V3CodebaseMemoryV2
        | CatalogProfile::V3CodebaseMemoryV2WriterLeaseV1
        | CatalogProfile::V3CodebaseMemoryV2WriterLeaseV2Bridge => 46,
        CatalogProfile::V4 => 40,
        // The autonomy receipt FK adds two enabled internal triggers on the
        // referenced task_ledger_events table; its two local triggers are
        // excluded with the autonomy relation itself above.
        CatalogProfile::V5 => 42,
        CatalogProfile::V5CodebaseMemoryV2UpgradePending
        | CatalogProfile::V5CodebaseMemoryV3Current
        | CatalogProfile::V5CodebaseMemoryV2WriterLeaseV2BridgePending
        | CatalogProfile::V5CodebaseMemoryV3WriterLeaseV2BridgePending
        | CatalogProfile::V5CodebaseMemoryV3WriterLeaseV2Current => 66,
    }
}

#[allow(clippy::too_many_lines)]
fn verify_owned_function_boundary<C: GenericClient>(
    client: &mut C,
    profile: CatalogProfile,
    v3_prefix: bool,
) -> Result<(), PostgresStoreSetupError> {
    let signature = catalog_signature(
        client,
        FUNCTION_SIGNATURE_SQL,
        PostgresStoreSetupErrorKind::CorruptCatalog,
    )?;
    let expected_signature = match profile {
        CatalogProfile::V2 => V2_EXPECTED_FUNCTION_SIGNATURE,
        CatalogProfile::V3 if v3_prefix => V3_PREFIX_EXPECTED_FUNCTION_SIGNATURE,
        CatalogProfile::V3 => V3_EXPECTED_FUNCTION_SIGNATURE,
        CatalogProfile::V3CodebaseMemoryV2
        | CatalogProfile::V3CodebaseMemoryV2WriterLeaseV1
        | CatalogProfile::V3CodebaseMemoryV2WriterLeaseV2Bridge
            if v3_prefix =>
        {
            V3_CODEBASE_MEMORY_V2_PREFIX_EXPECTED_FUNCTION_SIGNATURE
        }
        CatalogProfile::V3CodebaseMemoryV2
        | CatalogProfile::V3CodebaseMemoryV2WriterLeaseV1
        | CatalogProfile::V3CodebaseMemoryV2WriterLeaseV2Bridge => {
            V3_CODEBASE_MEMORY_V2_EXPECTED_FUNCTION_SIGNATURE
        }
        CatalogProfile::V4 => V4_EXPECTED_FUNCTION_SIGNATURE,
        CatalogProfile::V5 => V5_EXPECTED_FUNCTION_SIGNATURE,
        CatalogProfile::V5CodebaseMemoryV2UpgradePending
        | CatalogProfile::V5CodebaseMemoryV2WriterLeaseV2BridgePending => {
            V5_CODEBASE_MEMORY_V2_EXPECTED_FUNCTION_SIGNATURE
        }
        CatalogProfile::V5CodebaseMemoryV3Current
        | CatalogProfile::V5CodebaseMemoryV3WriterLeaseV2BridgePending
        | CatalogProfile::V5CodebaseMemoryV3WriterLeaseV2Current => {
            V5_CODEBASE_MEMORY_V3_EXPECTED_FUNCTION_SIGNATURE
        }
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
        CatalogProfile::V3CodebaseMemoryV2
        | CatalogProfile::V3CodebaseMemoryV2WriterLeaseV1
        | CatalogProfile::V3CodebaseMemoryV2WriterLeaseV2Bridge => [
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
        CatalogProfile::V5 => V3_CONTROL_FUNCTION_IDENTITIES
            .into_iter()
            .chain(V4_RUNTIME_FUNCTION_IDENTITIES)
            .chain(V5_SUCCESSOR_FUNCTION_IDENTITIES)
            .map(str::to_owned)
            .collect(),
        CatalogProfile::V5CodebaseMemoryV2UpgradePending
        | CatalogProfile::V5CodebaseMemoryV2WriterLeaseV2BridgePending => {
            V3_CONTROL_FUNCTION_IDENTITIES
                .into_iter()
                .chain(V4_RUNTIME_FUNCTION_IDENTITIES)
                .chain(V5_SUCCESSOR_FUNCTION_IDENTITIES)
                .chain(CODEBASE_MEMORY_V2_FUNCTION_IDENTITIES)
                .map(str::to_owned)
                .collect()
        }
        CatalogProfile::V5CodebaseMemoryV3Current
        | CatalogProfile::V5CodebaseMemoryV3WriterLeaseV2BridgePending
        | CatalogProfile::V5CodebaseMemoryV3WriterLeaseV2Current => V3_CONTROL_FUNCTION_IDENTITIES
            .into_iter()
            .chain(V4_RUNTIME_FUNCTION_IDENTITIES)
            .chain(V5_SUCCESSOR_FUNCTION_IDENTITIES)
            .chain(CODEBASE_MEMORY_V2_FUNCTION_IDENTITIES)
            .chain(CODEBASE_MEMORY_V3_FUNCTION_IDENTITIES)
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
               AND p.proname NOT IN ('task_ledger_record_autonomy_receipt_v1', \
                                     'task_ledger_read_autonomy_receipts_v1') \
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
            || V5_SUCCESSOR_FUNCTION_IDENTITIES.contains(&identity.as_str())
            || CODEBASE_MEMORY_V3_FUNCTION_IDENTITIES.contains(&identity.as_str())
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
              WHERE n.nspname IN ('control', 'memory', 'readmodel')), \
             (SELECT count(*) FROM pg_cast c \
               JOIN pg_type source_type ON source_type.oid=c.castsource \
               JOIN pg_namespace source_ns ON source_ns.oid=source_type.typnamespace \
               JOIN pg_type target_type ON target_type.oid=c.casttarget \
               JOIN pg_namespace target_ns ON target_ns.oid=target_type.typnamespace \
               LEFT JOIN pg_proc function_proc ON function_proc.oid=c.castfunc \
               LEFT JOIN pg_namespace function_ns ON function_ns.oid=function_proc.pronamespace \
              WHERE source_ns.nspname IN ('control', 'memory', 'readmodel') \
                 OR target_ns.nspname IN ('control', 'memory', 'readmodel') \
                 OR function_ns.nspname IN ('control', 'memory', 'readmodel')), \
             (SELECT count(*) FROM pg_transform tr \
               JOIN pg_type transformed_type ON transformed_type.oid=tr.trftype \
               JOIN pg_namespace transformed_ns ON transformed_ns.oid=transformed_type.typnamespace \
               LEFT JOIN pg_proc from_proc ON from_proc.oid=tr.trffromsql \
               LEFT JOIN pg_namespace from_ns ON from_ns.oid=from_proc.pronamespace \
               LEFT JOIN pg_proc to_proc ON to_proc.oid=tr.trftosql \
               LEFT JOIN pg_namespace to_ns ON to_ns.oid=to_proc.pronamespace \
              WHERE transformed_ns.nspname IN ('control', 'memory', 'readmodel') \
                 OR from_ns.nspname IN ('control', 'memory', 'readmodel') \
                 OR to_ns.nspname IN ('control', 'memory', 'readmodel'))",
            &[],
        )
        .map_err(|error| map_postgres_error(&error, PostgresStoreSetupErrorKind::CorruptCatalog))?;
    for index in 0..12 {
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
    verify_roles_and_grants_with_contract(client, profile, false)
}

fn verify_roles_and_grants_with_contract<C: GenericClient>(
    client: &mut C,
    profile: CatalogProfile,
    v3_prefix: bool,
) -> Result<(), PostgresStoreSetupError> {
    verify_role_and_database_boundary(client, profile, v3_prefix)?;
    let expected_schema_acl = match profile {
        CatalogProfile::V3CodebaseMemoryV2
        | CatalogProfile::V3CodebaseMemoryV2WriterLeaseV1
        | CatalogProfile::V3CodebaseMemoryV2WriterLeaseV2Bridge
        | CatalogProfile::V5CodebaseMemoryV2UpgradePending
        | CatalogProfile::V5CodebaseMemoryV3Current
        | CatalogProfile::V5CodebaseMemoryV2WriterLeaseV2BridgePending
        | CatalogProfile::V5CodebaseMemoryV3WriterLeaseV2BridgePending
        | CatalogProfile::V5CodebaseMemoryV3WriterLeaseV2Current => {
            V3_CODEBASE_MEMORY_V2_EXPECTED_SCHEMA_ACL_SIGNATURE
        }
        CatalogProfile::PreSchema
        | CatalogProfile::V1
        | CatalogProfile::V2
        | CatalogProfile::V3
        | CatalogProfile::V4
        | CatalogProfile::V5 => EXPECTED_SCHEMA_ACL_SIGNATURE,
    };
    let expected_table_acl = match profile {
        CatalogProfile::PreSchema | CatalogProfile::V1 | CatalogProfile::V2 => {
            EXPECTED_TABLE_ACL_SIGNATURE
        }
        CatalogProfile::V3 => V3_EXPECTED_TABLE_ACL_SIGNATURE,
        CatalogProfile::V3CodebaseMemoryV2
        | CatalogProfile::V3CodebaseMemoryV2WriterLeaseV1
        | CatalogProfile::V3CodebaseMemoryV2WriterLeaseV2Bridge => {
            V3_CODEBASE_MEMORY_V2_EXPECTED_TABLE_ACL_SIGNATURE
        }
        CatalogProfile::V4 | CatalogProfile::V5 => V4_EXPECTED_TABLE_ACL_SIGNATURE,
        CatalogProfile::V5CodebaseMemoryV2UpgradePending
        | CatalogProfile::V5CodebaseMemoryV2WriterLeaseV2BridgePending => {
            V5_CODEBASE_MEMORY_V2_EXPECTED_TABLE_ACL_SIGNATURE
        }
        CatalogProfile::V5CodebaseMemoryV3Current
        | CatalogProfile::V5CodebaseMemoryV3WriterLeaseV2BridgePending
        | CatalogProfile::V5CodebaseMemoryV3WriterLeaseV2Current => {
            V5_CODEBASE_MEMORY_V3_EXPECTED_TABLE_ACL_SIGNATURE
        }
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
            | CatalogProfile::V3CodebaseMemoryV2WriterLeaseV2Bridge
            | CatalogProfile::V4
            | CatalogProfile::V5
            | CatalogProfile::V5CodebaseMemoryV2UpgradePending
            | CatalogProfile::V5CodebaseMemoryV3Current
            | CatalogProfile::V5CodebaseMemoryV2WriterLeaseV2BridgePending
            | CatalogProfile::V5CodebaseMemoryV3WriterLeaseV2BridgePending
            | CatalogProfile::V5CodebaseMemoryV3WriterLeaseV2Current
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
        CatalogProfile::V3CodebaseMemoryV2
        | CatalogProfile::V3CodebaseMemoryV2WriterLeaseV1
        | CatalogProfile::V3CodebaseMemoryV2WriterLeaseV2Bridge => {
            V3_CODEBASE_MEMORY_V2_EXPECTED_FUNCTION_ACL_SIGNATURE
        }
        CatalogProfile::V4 => V4_EXPECTED_FUNCTION_ACL_SIGNATURE,
        CatalogProfile::V5 => V5_EXPECTED_FUNCTION_ACL_SIGNATURE,
        CatalogProfile::V5CodebaseMemoryV2UpgradePending
        | CatalogProfile::V5CodebaseMemoryV2WriterLeaseV2BridgePending => {
            V5_CODEBASE_MEMORY_V2_EXPECTED_FUNCTION_ACL_SIGNATURE
        }
        CatalogProfile::V5CodebaseMemoryV3Current
        | CatalogProfile::V5CodebaseMemoryV3WriterLeaseV2BridgePending
        | CatalogProfile::V5CodebaseMemoryV3WriterLeaseV2Current => {
            V5_CODEBASE_MEMORY_V3_EXPECTED_FUNCTION_ACL_SIGNATURE
        }
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
        CatalogProfile::V3CodebaseMemoryV2
        | CatalogProfile::V3CodebaseMemoryV2WriterLeaseV1
        | CatalogProfile::V3CodebaseMemoryV2WriterLeaseV2Bridge => [
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
        CatalogProfile::V5 => V5_SUCCESSOR_FUNCTION_IDENTITIES
            .into_iter()
            .map(str::to_owned)
            .collect(),
        CatalogProfile::V5CodebaseMemoryV2UpgradePending
        | CatalogProfile::V5CodebaseMemoryV2WriterLeaseV2BridgePending => {
            V5_SUCCESSOR_FUNCTION_IDENTITIES
                .into_iter()
                .chain(CODEBASE_MEMORY_V2_FUNCTION_IDENTITIES)
                .map(str::to_owned)
                .collect()
        }
        CatalogProfile::V5CodebaseMemoryV3Current
        | CatalogProfile::V5CodebaseMemoryV3WriterLeaseV2BridgePending
        | CatalogProfile::V5CodebaseMemoryV3WriterLeaseV2Current => {
            V5_SUCCESSOR_FUNCTION_IDENTITIES
                .into_iter()
                .chain(CODEBASE_MEMORY_V3_FUNCTION_IDENTITIES)
                .map(str::to_owned)
                .collect()
        }
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
               AND p.proname NOT IN ('task_ledger_record_autonomy_receipt_v1', \
                                     'task_ledger_read_autonomy_receipts_v1') \
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
    v3_prefix: bool,
) -> Result<(), PostgresStoreSetupError> {
    let dangerous_functions = verify_exact_principal_database_core(client)?;
    let expected_dangerous_functions = expected_dangerous_function_count(profile, v3_prefix);
    if dangerous_functions != expected_dangerous_functions {
        return Err(permission_error());
    }
    verify_cluster_wide_acl_closure(client, profile)
}

fn verify_exact_principal_database_core<C: GenericClient>(
    client: &mut C,
) -> Result<i64, PostgresStoreSetupError> {
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
        .query_one(ROLE_DATABASE_BOUNDARY_SQL, &[])
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
    if owner != DatabaseRole::Migrator.as_str()
        || is_template
        || !allows_connections
        || connection_limit != -1
        || memberships != 4
        || extra_roles != 0
        || role_settings != 0
        || database_privileges != [false, false, false, true, true, true]
    {
        return Err(permission_error());
    }
    verify_login_principal_closure(client)?;
    Ok(dangerous_functions)
}

/// SQL for the independently versioned, same-database Control product facts.
pub const CONTROL_PRODUCT_SQL: &str = include_str!("../../../db/extensions/control-product/v1.sql");
const CONTROL_PRODUCT_FUNCTION_CATALOG_SHA256: &str =
    "500622c2dc4cb24ca5bf2ed1b7d3bd537783754c6b8cba17fd1d790f3ec07b4d";
const CONTROL_PRODUCT_TABLE_CATALOG_SHA256: &str =
    "28c9a3ae3d9038332ab590ab073ba8385b3c4940d82cf54b097a1e7d087569b8";

struct ControlProductPrincipalProfile {
    relation_oids: Vec<i64>,
    function_oids: Vec<i64>,
}

#[allow(clippy::too_many_lines)]
fn verify_optional_control_product_extension<C: GenericClient>(
    client: &mut C,
) -> Result<Option<ControlProductPrincipalProfile>, PostgresStoreSetupError> {
    let present = client
        .query_one("SELECT to_regnamespace('control_product') IS NOT NULL", &[])
        .map_err(|error| map_postgres_error(&error, PostgresStoreSetupErrorKind::CorruptCatalog))?
        .get::<_, bool>(0);
    if !present {
        return Ok(None);
    }
    // Reuse the existing full catalog model for the second owned extension.
    // No functions or relation OIDs are exempted before their bytes and ACLs match.
    let functions = managed_foreman_catalog_digest(
        client,
        &MANAGED_FOREMAN_FUNCTION_CATALOG_SQL.replace("foreman_execution", "control_product"),
        b"LATTICE_CONTROL_PRODUCT_FUNCTION_CATALOG_V1\0",
    )?;
    let tables = managed_foreman_catalog_digest(
        client,
        &MANAGED_FOREMAN_TABLE_CATALOG_SQL.replace("foreman_execution", "control_product"),
        b"LATTICE_CONTROL_PRODUCT_TABLE_CATALOG_V1\0",
    )?;
    if functions != CONTROL_PRODUCT_FUNCTION_CATALOG_SHA256
        || tables != CONTROL_PRODUCT_TABLE_CATALOG_SHA256
    {
        return Err(catalog_error());
    }
    let shape = client.query_one(
        "SELECT (SELECT count(*) FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname='control_product'), \
          (SELECT count(*) FROM pg_proc p JOIN pg_namespace n ON n.oid=p.pronamespace WHERE n.nspname='control_product'), \
          (SELECT count(*) FROM pg_type t JOIN pg_namespace n ON n.oid=t.typnamespace WHERE n.nspname='control_product'), \
          (SELECT count(*) FROM pg_trigger t JOIN pg_class c ON c.oid=t.tgrelid JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname='control_product' AND NOT t.tgisinternal), \
          (SELECT count(*) FROM pg_policy p JOIN pg_class c ON c.oid=p.polrelid JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname='control_product'), \
          (SELECT count(*) FROM pg_rewrite r JOIN pg_class c ON c.oid=r.ev_class JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname='control_product')",
        &[]).map_err(|error| map_postgres_error(&error, PostgresStoreSetupErrorKind::CorruptCatalog))?;
    for (index, expected) in [(0, 26_i64), (1, 15), (2, 16), (3, 0), (4, 0), (5, 0)] {
        if row_value::<i64>(&shape, index, PostgresStoreSetupErrorKind::CorruptCatalog)? != expected
        {
            return Err(catalog_error());
        }
    }
    let identity = client
        .query("SELECT * FROM control_product.identity_read_v1()", &[])
        .map_err(|error| map_postgres_error(&error, PostgresStoreSetupErrorKind::CorruptCatalog))?;
    if identity.len() != 1 {
        return Err(catalog_error());
    }
    let identity = &identity[0];
    let uuid = row_value::<String>(identity, 0, PostgresStoreSetupErrorKind::CorruptCatalog)?;
    let manifest = row_value::<String>(identity, 1, PostgresStoreSetupErrorKind::CorruptCatalog)?;
    let sql_digest = row_value::<String>(identity, 2, PostgresStoreSetupErrorKind::CorruptCatalog)?;
    if uuid != row_value::<String>(identity, 3, PostgresStoreSetupErrorKind::CorruptCatalog)?
        || manifest
            != row_value::<String>(identity, 4, PostgresStoreSetupErrorKind::CorruptCatalog)?
        || manifest != CURRENT_V8_MANIFEST_SHA256
        || sql_digest != hex_digest(&Sha256::digest(CONTROL_PRODUCT_SQL.as_bytes()))
    {
        return Err(catalog_error());
    }
    let relation_oids = client.query("SELECT c.oid::bigint FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname='control_product' ORDER BY c.oid", &[])
        .map_err(|error| map_postgres_error(&error, PostgresStoreSetupErrorKind::CorruptCatalog))?
        .iter().map(|row| row.get(0)).collect();
    let function_oids = client.query("SELECT p.oid::bigint FROM pg_proc p JOIN pg_namespace n ON n.oid=p.pronamespace WHERE n.nspname='control_product' ORDER BY p.oid", &[])
        .map_err(|error| map_postgres_error(&error, PostgresStoreSetupErrorKind::CorruptCatalog))?
        .iter().map(|row| row.get(0)).collect();
    Ok(Some(ControlProductPrincipalProfile {
        relation_oids,
        function_oids,
    }))
}

/// Installs Control facts only through an explicitly invoked, verified migrator.
/// Existing or partially installed extensions are verified rather than overwritten.
///
/// # Errors
/// Rejects incompatible Store state, catalog drift, or unavailable persistence.
pub fn apply_control_product_extension(
    client: &mut Client,
    target: &MigrationTarget,
) -> Result<(), PostgresStoreSetupError> {
    let current = verify_postgres_schema(client, target, DatabaseRole::Migrator)?;
    if current.schema_version() != 8 {
        return Err(catalog_error());
    }
    let mut transaction = client
        .build_transaction()
        .isolation_level(IsolationLevel::Serializable)
        .start()
        .map_err(|error| {
            map_postgres_error(&error, PostgresStoreSetupErrorKind::TransactionFailed)
        })?;
    harden_transaction(&mut transaction)?;
    let present = transaction
        .query_one("SELECT to_regnamespace('control_product') IS NOT NULL", &[])
        .map_err(|error| map_postgres_error(&error, PostgresStoreSetupErrorKind::CorruptCatalog))?
        .get::<_, bool>(0);
    if !present {
        transaction
            .batch_execute(CONTROL_PRODUCT_SQL)
            .map_err(|error| {
                map_postgres_error(&error, PostgresStoreSetupErrorKind::TransactionFailed)
            })?;
        let sql_digest = hex_digest(&Sha256::digest(CONTROL_PRODUCT_SQL.as_bytes()));
        transaction.execute("INSERT INTO control_product.extension_identity(singleton,database_uuid,store_manifest,sql_sha256) VALUES(true,$1::text::uuid,$2,$3)",
            &[&current.database_uuid(),&current.manifest_sha256().as_str(),&sql_digest])
            .map_err(|error| map_postgres_error(&error,PostgresStoreSetupErrorKind::TransactionFailed))?;
    }
    verify_optional_control_product_extension(&mut transaction)?.ok_or_else(catalog_error)?;
    transaction.commit().map_err(|error| {
        map_postgres_error(&error, PostgresStoreSetupErrorKind::TransactionFailed)
    })?;
    verify_postgres_schema(client, target, DatabaseRole::Migrator)?;
    Ok(())
}

fn verify_exact_principal_database_boundary<C: GenericClient>(
    client: &mut C,
    expected_dangerous_functions: i64,
    writer_lease_is_owned: bool,
    managed_foreman: Option<&ManagedForemanPrincipalProfile>,
) -> Result<(), PostgresStoreSetupError> {
    let product = verify_optional_control_product_extension(client)?;
    let product_functions = if product.is_some() { 14 } else { 0 };
    if verify_exact_principal_database_core(client)?
        != expected_dangerous_functions + product_functions
    {
        return Err(permission_error());
    }
    verify_cluster_wide_acl_closure_for_owned_extensions(
        client,
        writer_lease_is_owned,
        managed_foreman,
        product.as_ref(),
    )
}

fn expected_dangerous_function_count(profile: CatalogProfile, v3_prefix: bool) -> i64 {
    match profile {
        CatalogProfile::PreSchema | CatalogProfile::V1 => 0,
        CatalogProfile::V2 => 3,
        CatalogProfile::V3 if v3_prefix => 8,
        CatalogProfile::V3 => 10,
        CatalogProfile::V3CodebaseMemoryV2
        | CatalogProfile::V3CodebaseMemoryV2WriterLeaseV2Bridge
            if v3_prefix =>
        {
            15
        }
        CatalogProfile::V3CodebaseMemoryV2
        | CatalogProfile::V4
        | CatalogProfile::V3CodebaseMemoryV2WriterLeaseV2Bridge => 17,
        CatalogProfile::V3CodebaseMemoryV2WriterLeaseV1 if v3_prefix => 22,
        CatalogProfile::V3CodebaseMemoryV2WriterLeaseV1 => 24,
        CatalogProfile::V5 => {
            i64::try_from(V5_RUNTIME_FUNCTION_IDENTITIES.len()).expect("fixed count")
        }
        CatalogProfile::V5CodebaseMemoryV2UpgradePending
        | CatalogProfile::V5CodebaseMemoryV3Current
        | CatalogProfile::V5CodebaseMemoryV2WriterLeaseV2BridgePending
        | CatalogProfile::V5CodebaseMemoryV3WriterLeaseV2BridgePending => i64::try_from(
            V5_RUNTIME_FUNCTION_IDENTITIES.len() + CODEBASE_MEMORY_V2_FUNCTION_IDENTITIES.len(),
        )
        .expect("fixed count"),
        CatalogProfile::V5CodebaseMemoryV3WriterLeaseV2Current => i64::try_from(
            V5_RUNTIME_FUNCTION_IDENTITIES.len()
                + CODEBASE_MEMORY_V2_FUNCTION_IDENTITIES.len()
                + WRITER_LEASE_V1_FUNCTIONS.len(),
        )
        .expect("fixed count"),
    }
}

fn verify_cluster_wide_acl_closure<C: GenericClient>(
    client: &mut C,
    profile: CatalogProfile,
) -> Result<(), PostgresStoreSetupError> {
    let writer_lease_is_owned = matches!(
        profile,
        CatalogProfile::V3CodebaseMemoryV2WriterLeaseV1
            | CatalogProfile::V3CodebaseMemoryV2WriterLeaseV2Bridge
            | CatalogProfile::V5CodebaseMemoryV2WriterLeaseV2BridgePending
            | CatalogProfile::V5CodebaseMemoryV3WriterLeaseV2BridgePending
            | CatalogProfile::V5CodebaseMemoryV3WriterLeaseV2Current
    );
    verify_cluster_wide_acl_closure_for_owned_extensions(client, writer_lease_is_owned, None, None)
}

fn verify_cluster_wide_acl_closure_for_owned_extensions<C: GenericClient>(
    client: &mut C,
    writer_lease_is_owned: bool,
    managed_foreman: Option<&ManagedForemanPrincipalProfile>,
    product: Option<&ControlProductPrincipalProfile>,
) -> Result<(), PostgresStoreSetupError> {
    let parameter_grants = client
        .query_one(
            "SELECT count(*) FROM pg_parameter_acl p \
             CROSS JOIN LATERAL aclexplode(CASE \
                 WHEN cardinality(p.paracl)=0 THEN NULL::aclitem[] \
                 ELSE p.paracl END) acl \
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
             CROSS JOIN LATERAL aclexplode(CASE \
                 WHEN cardinality(COALESCE(d.datacl, acldefault('d', d.datdba)))=0 \
                   THEN NULL::aclitem[] \
                 ELSE COALESCE(d.datacl, acldefault('d', d.datdba)) END) acl \
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
    verify_managed_extension_dependency_closure(
        client,
        writer_lease_is_owned,
        managed_foreman.is_some(),
        product.is_some(),
    )?;
    verify_external_relation_principal_closure(
        client,
        writer_lease_is_owned,
        managed_foreman,
        product,
    )?;
    verify_external_function_principal_closure(
        client,
        writer_lease_is_owned,
        managed_foreman,
        product,
    )?;
    verify_pre_role_system_function_boundary(client)?;
    verify_large_object_boundary(client)
}

#[allow(clippy::too_many_lines)]
fn verify_managed_extension_dependency_closure<C: GenericClient>(
    client: &mut C,
    writer_lease_is_owned: bool,
    foreman_is_owned: bool,
    product_is_owned: bool,
) -> Result<(), PostgresStoreSetupError> {
    let forbidden = client
        .query_one(
            "WITH managed_namespaces(objid) AS ( \
                SELECT n.oid FROM pg_namespace n \
                 WHERE n.nspname IN ('control','memory','readmodel') \
                    OR ($1 AND n.nspname='writer_lease') \
                    OR ($2 AND n.nspname='foreman_execution') \
                    OR ($3 AND n.nspname='control_product') \
            ), managed_relations(objid) AS ( \
                SELECT c.oid FROM pg_class c \
                 WHERE c.relnamespace IN (SELECT objid FROM managed_namespaces) \
            ), managed_constraints(objid) AS ( \
                SELECT c.oid FROM pg_constraint c \
                 WHERE c.connamespace IN (SELECT objid FROM managed_namespaces) \
            ), managed_toast_relations(objid) AS ( \
                SELECT c.reltoastrelid FROM pg_class c \
                 WHERE c.oid IN (SELECT objid FROM managed_relations) \
                   AND c.reltoastrelid<>0 \
                UNION \
                SELECT i.indexrelid FROM pg_index i \
                 WHERE i.indrelid IN ( \
                    SELECT c.reltoastrelid FROM pg_class c \
                     WHERE c.oid IN (SELECT objid FROM managed_relations) \
                       AND c.reltoastrelid<>0) \
            ), managed_functions(objid) AS ( \
                SELECT p.oid FROM pg_proc p \
                 WHERE p.pronamespace IN (SELECT objid FROM managed_namespaces) \
            ), managed_types(objid) AS ( \
                SELECT t.oid FROM pg_type t \
                 WHERE t.typnamespace IN (SELECT objid FROM managed_namespaces) \
                    OR t.typrelid IN (SELECT objid FROM managed_toast_relations) \
            ), managed_casts(objid) AS ( \
                SELECT c.oid FROM pg_cast c \
                 WHERE c.castsource IN (SELECT objid FROM managed_types) \
                    OR c.casttarget IN (SELECT objid FROM managed_types) \
                    OR c.castfunc IN (SELECT objid FROM managed_functions) \
            ), managed_transforms(objid) AS ( \
                SELECT tr.oid FROM pg_transform tr \
                 WHERE tr.trftype IN (SELECT objid FROM managed_types) \
                    OR tr.trffromsql IN (SELECT objid FROM managed_functions) \
                    OR tr.trftosql IN (SELECT objid FROM managed_functions) \
            ), managed(classid,objid) AS ( \
                SELECT 'pg_namespace'::regclass::oid,objid FROM managed_namespaces \
                UNION \
                SELECT 'pg_class'::regclass::oid,objid FROM managed_relations \
                UNION \
                SELECT 'pg_class'::regclass::oid,objid FROM managed_toast_relations \
                UNION \
                SELECT 'pg_proc'::regclass::oid,objid FROM managed_functions \
                UNION \
                SELECT 'pg_type'::regclass::oid,objid FROM managed_types \
                UNION \
                SELECT 'pg_cast'::regclass::oid,objid FROM managed_casts \
                UNION \
                SELECT 'pg_transform'::regclass::oid,objid FROM managed_transforms \
                UNION \
                SELECT 'pg_constraint'::regclass::oid,objid FROM managed_constraints \
                UNION \
                SELECT 'pg_attrdef'::regclass::oid,a.oid FROM pg_attrdef a \
                 WHERE a.adrelid IN (SELECT objid FROM managed_relations) \
                UNION \
                SELECT 'pg_trigger'::regclass::oid,t.oid FROM pg_trigger t \
                 WHERE t.tgrelid IN (SELECT objid FROM managed_relations) \
                    OR t.tgconstraint IN (SELECT objid FROM managed_constraints) \
                UNION \
                SELECT 'pg_rewrite'::regclass::oid,r.oid FROM pg_rewrite r \
                 WHERE r.ev_class IN (SELECT objid FROM managed_relations) \
                UNION \
                SELECT 'pg_policy'::regclass::oid,p.oid FROM pg_policy p \
                 WHERE p.polrelid IN (SELECT objid FROM managed_relations) \
                UNION \
                SELECT 'pg_statistic_ext'::regclass::oid,s.oid FROM pg_statistic_ext s \
                 WHERE s.stxnamespace IN (SELECT objid FROM managed_namespaces) \
            ) \
            SELECT count(*) FROM pg_depend d \
              JOIN managed dependent \
                ON dependent.classid=d.classid AND dependent.objid=d.objid \
             WHERE d.deptype IN ('e','x')",
            &[&writer_lease_is_owned, &foreman_is_owned, &product_is_owned],
        )
        .map_err(|error| map_postgres_error(&error, PostgresStoreSetupErrorKind::CorruptCatalog))?;
    if row_value::<i64>(&forbidden, 0, PostgresStoreSetupErrorKind::CorruptCatalog)? != 0 {
        return Err(catalog_error());
    }
    Ok(())
}

fn verify_external_relation_principal_closure<C: GenericClient>(
    client: &mut C,
    writer_lease_is_owned: bool,
    managed_foreman: Option<&ManagedForemanPrincipalProfile>,
    product: Option<&ControlProductPrincipalProfile>,
) -> Result<(), PostgresStoreSetupError> {
    let mut foreman_relation_oids = managed_foreman
        .map(|profile| profile.relation_oids.clone())
        .unwrap_or_default();
    if let Some(product) = product {
        foreman_relation_oids.extend_from_slice(&product.relation_oids);
    }
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
                   AND c.oid::bigint <> ALL($2::bigint[]) \
             ) \
             SELECT \
               (SELECT count(*) FROM external_relations c \
                WHERE c.relowner IN (SELECT oid FROM fixed_principals)), \
               (SELECT count(*) FROM external_relations c \
                CROSS JOIN LATERAL aclexplode(CASE \
                    WHEN cardinality(c.relacl)=0 THEN NULL::aclitem[] \
                    ELSE c.relacl END) acl \
                WHERE acl.grantee = 0 \
                   OR acl.grantee IN (SELECT oid FROM fixed_principals)), \
               (SELECT count(*) FROM pg_attribute a \
                JOIN external_relations c ON c.oid = a.attrelid \
                CROSS JOIN LATERAL aclexplode(CASE \
                    WHEN cardinality(a.attacl)=0 THEN NULL::aclitem[] \
                    ELSE a.attacl END) acl \
                WHERE acl.grantee = 0 \
                   OR acl.grantee IN (SELECT oid FROM fixed_principals))",
            &[&writer_lease_is_owned, &foreman_relation_oids],
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
    managed_foreman: Option<&ManagedForemanPrincipalProfile>,
    product: Option<&ControlProductPrincipalProfile>,
) -> Result<(), PostgresStoreSetupError> {
    let mut foreman_function_oids = managed_foreman
        .map(|profile| profile.function_oids.clone())
        .unwrap_or_default();
    if let Some(product) = product {
        foreman_function_oids.extend_from_slice(&product.function_oids);
    }
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
             LEFT JOIN LATERAL aclexplode( \
                 CASE \
                   WHEN cardinality(COALESCE(p.proacl, acldefault('f', p.proowner)))=0 \
                     THEN NULL::aclitem[] \
                   ELSE COALESCE(p.proacl, acldefault('f', p.proowner)) \
                 END \
             ) acl ON TRUE \
             WHERE n.nspname !~ '^pg_' \
               AND n.nspname <> 'information_schema' \
               AND n.nspname NOT IN ('control', 'memory', 'readmodel') \
               AND (NOT $1 OR n.nspname <> 'writer_lease') \
               AND p.oid::bigint <> ALL($2::bigint[]) \
               AND (p.proowner IN (SELECT oid FROM fixed_principals) \
                    OR acl.grantee = 0 \
                    OR acl.grantee IN (SELECT oid FROM fixed_principals))",
            &[&writer_lease_is_owned, &foreman_function_oids],
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
                 ('pg_catalog.pg_try_advisory_lock(bigint)', 'lattice_migrator'::text), \
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
                CROSS JOIN LATERAL aclexplode(CASE \
                    WHEN cardinality(COALESCE(p.proacl, acldefault('f', p.proowner)))=0 \
                      THEN NULL::aclitem[] \
                    ELSE COALESCE(p.proacl, acldefault('f', p.proowner)) END \
                ) acl WHERE acl.grantee = 0), \
               (SELECT count(*) FROM resolved r CROSS JOIN fixed_roles f \
                WHERE has_function_privilege(f.role_name, r.function_oid, 'EXECUTE') \
                    <> COALESCE(r.allowed_role = f.role_name, false)), \
               (SELECT count(*) FROM resolved r \
                JOIN pg_proc p ON p.oid = r.function_oid \
                WHERE (SELECT count(*) \
                       FROM aclexplode(CASE \
                           WHEN cardinality(COALESCE(p.proacl, acldefault('f', p.proowner)))=0 \
                             THEN NULL::aclitem[] \
                           ELSE COALESCE(p.proacl, acldefault('f', p.proowner)) END) acl \
                       JOIN pg_roles grantee ON grantee.oid = acl.grantee \
                       WHERE grantee.rolname IN ('lattice_migrator', 'lattice_runtime', \
                           'lattice_guardian', 'lattice_readonly', \
                           'lattice_migrator_login', 'lattice_runtime_login', \
                           'lattice_guardian_login', 'lattice_readonly_login')) \
                       <> CASE WHEN r.allowed_role IS NULL THEN 0 ELSE 1 END \
                   OR (r.allowed_role IS NOT NULL AND NOT EXISTS ( \
                       SELECT 1 \
                       FROM aclexplode(CASE \
                           WHEN cardinality(COALESCE(p.proacl, acldefault('f', p.proowner)))=0 \
                             THEN NULL::aclitem[] \
                           ELSE COALESCE(p.proacl, acldefault('f', p.proowner)) END) acl \
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
                CROSS JOIN LATERAL aclexplode(CASE \
                    WHEN cardinality(l.lomacl)=0 THEN NULL::aclitem[] \
                    ELSE l.lomacl END) acl \
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
                CROSS JOIN LATERAL aclexplode(CASE \
                    WHEN cardinality(COALESCE(n.nspacl, acldefault('n', n.nspowner)))=0 \
                      THEN NULL::aclitem[] \
                    ELSE COALESCE(n.nspacl, acldefault('n', n.nspowner)) END) acl \
                UNION ALL SELECT acl.grantee FROM pg_class c \
                CROSS JOIN LATERAL aclexplode(CASE \
                    WHEN cardinality(COALESCE(c.relacl, acldefault('r', c.relowner)))=0 \
                      THEN NULL::aclitem[] \
                    ELSE COALESCE(c.relacl, acldefault('r', c.relowner)) END) acl \
                UNION ALL SELECT acl.grantee FROM pg_proc p \
                CROSS JOIN LATERAL aclexplode(CASE \
                    WHEN cardinality(COALESCE(p.proacl, acldefault('f', p.proowner)))=0 \
                      THEN NULL::aclitem[] \
                    ELSE COALESCE(p.proacl, acldefault('f', p.proowner)) END) acl \
                UNION ALL SELECT acl.grantee FROM pg_type t \
                CROSS JOIN LATERAL aclexplode(CASE \
                    WHEN cardinality(COALESCE(t.typacl, acldefault('T', t.typowner)))=0 \
                      THEN NULL::aclitem[] \
                    ELSE COALESCE(t.typacl, acldefault('T', t.typowner)) END) acl \
                UNION ALL SELECT acl.grantee FROM pg_attribute a \
                CROSS JOIN LATERAL aclexplode(CASE \
                    WHEN cardinality(a.attacl)=0 THEN NULL::aclitem[] \
                    ELSE a.attacl END) acl \
                UNION ALL SELECT acl.grantee FROM pg_language l \
                CROSS JOIN LATERAL aclexplode(CASE \
                    WHEN cardinality(l.lanacl)=0 THEN NULL::aclitem[] \
                    ELSE l.lanacl END) acl \
                UNION ALL SELECT acl.grantee FROM pg_foreign_data_wrapper f \
                CROSS JOIN LATERAL aclexplode(CASE \
                    WHEN cardinality(f.fdwacl)=0 THEN NULL::aclitem[] \
                    ELSE f.fdwacl END) acl \
                UNION ALL SELECT acl.grantee FROM pg_foreign_server s \
                CROSS JOIN LATERAL aclexplode(CASE \
                    WHEN cardinality(s.srvacl)=0 THEN NULL::aclitem[] \
                    ELSE s.srvacl END) acl \
                UNION ALL SELECT acl.grantee FROM pg_tablespace s \
                CROSS JOIN LATERAL aclexplode(CASE \
                    WHEN cardinality(s.spcacl)=0 THEN NULL::aclitem[] \
                    ELSE s.spcacl END) acl \
                UNION ALL SELECT acl.grantee FROM pg_largeobject_metadata l \
                CROSS JOIN LATERAL aclexplode(CASE \
                    WHEN cardinality(l.lomacl)=0 THEN NULL::aclitem[] \
                    ELSE l.lomacl END) acl \
                UNION ALL SELECT acl.grantee FROM pg_default_acl d \
                CROSS JOIN LATERAL aclexplode(CASE \
                    WHEN cardinality(d.defaclacl)=0 THEN NULL::aclitem[] \
                    ELSE d.defaclacl END) acl \
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
             CROSS JOIN LATERAL aclexplode(CASE \
                 WHEN cardinality(COALESCE(d.datacl, acldefault('d', d.datdba)))=0 \
                   THEN NULL::aclitem[] \
                 ELSE COALESCE(d.datacl, acldefault('d', d.datdba)) END) acl \
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

// This is one fail-closed capability matrix: splitting it would make it easy
// to admit a role after checking only its schema or table half.
#[allow(clippy::too_many_lines)]
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
            CatalogProfile::V3CodebaseMemoryV2
                | CatalogProfile::V3CodebaseMemoryV2WriterLeaseV1
                | CatalogProfile::V3CodebaseMemoryV2WriterLeaseV2Bridge
                | CatalogProfile::V5CodebaseMemoryV2UpgradePending
                | CatalogProfile::V5CodebaseMemoryV3Current
                | CatalogProfile::V5CodebaseMemoryV2WriterLeaseV2BridgePending
                | CatalogProfile::V5CodebaseMemoryV3WriterLeaseV2BridgePending
                | CatalogProfile::V5CodebaseMemoryV3WriterLeaseV2Current
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
        | CatalogProfile::V3CodebaseMemoryV2WriterLeaseV1
        | CatalogProfile::V3CodebaseMemoryV2WriterLeaseV2Bridge => {
            V3_PROTECTED_CONTROL_TABLES.into_iter().collect()
        }
        CatalogProfile::V4
        | CatalogProfile::V5
        | CatalogProfile::V5CodebaseMemoryV2UpgradePending
        | CatalogProfile::V5CodebaseMemoryV3Current
        | CatalogProfile::V5CodebaseMemoryV2WriterLeaseV2BridgePending
        | CatalogProfile::V5CodebaseMemoryV3WriterLeaseV2BridgePending
        | CatalogProfile::V5CodebaseMemoryV3WriterLeaseV2Current => {
            V4_PROTECTED_CONTROL_TABLES.into_iter().collect()
        }
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
        CatalogProfile::V3CodebaseMemoryV2
            | CatalogProfile::V3CodebaseMemoryV2WriterLeaseV1
            | CatalogProfile::V3CodebaseMemoryV2WriterLeaseV2Bridge
            | CatalogProfile::V5CodebaseMemoryV2UpgradePending
            | CatalogProfile::V5CodebaseMemoryV3Current
            | CatalogProfile::V5CodebaseMemoryV2WriterLeaseV2BridgePending
            | CatalogProfile::V5CodebaseMemoryV3WriterLeaseV2BridgePending
            | CatalogProfile::V5CodebaseMemoryV3WriterLeaseV2Current
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
                 CASE \
                   WHEN cardinality(COALESCE(d.defaclacl, \
                       acldefault(expected.acldefault_type, r.oid)))=0 \
                     THEN NULL::aclitem[] \
                   ELSE COALESCE(d.defaclacl, \
                       acldefault(expected.acldefault_type, r.oid)) \
                 END \
             ) a \
             WHERE a.grantee = 0), \
             (SELECT count(*) FROM pg_default_acl d \
             JOIN pg_namespace n ON n.oid = d.defaclnamespace \
             CROSS JOIN LATERAL aclexplode(CASE \
                 WHEN cardinality(d.defaclacl)=0 THEN NULL::aclitem[] \
                 ELSE d.defaclacl END) a \
             WHERE d.defaclrole = 'lattice_migrator'::regrole \
             AND n.nspname IN ('control', 'memory', 'readmodel') \
             AND a.grantee = 0), \
             (SELECT count(*) FROM pg_default_acl d \
              CROSS JOIN LATERAL aclexplode(CASE \
                  WHEN cardinality(d.defaclacl)=0 THEN NULL::aclitem[] \
                  ELSE d.defaclacl END) a \
              WHERE a.grantee = 0), \
             (SELECT count(*) FROM pg_default_acl d \
              WHERE d.defaclrole <> 'lattice_migrator'::regrole)",
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
    let non_migrator_defaults =
        row_value::<i64>(&defaults, 3, PostgresStoreSetupErrorKind::PermissionDenied)?;
    if global_public_defaults != 0
        || schema_public_defaults != 0
        || all_owner_public_defaults != 0
        || non_migrator_defaults != 0
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
        AUTONOMY_PROFILE_SIGNATURE_SQL, CODEBASE_MEMORY_EXTENSION_ID,
        CODEBASE_MEMORY_V3_GLOBAL_SCHEMA_VERSION, CODEBASE_MEMORY_V3_MANIFEST_SHA256,
        CODEBASE_MEMORY_V3_PATH, CODEBASE_MEMORY_V3_SCHEMA_VERSION, CODEBASE_MEMORY_V3_SQL_SHA256,
        COLUMN_SIGNATURE_SQL, CONSTRAINT_SIGNATURE_SQL, CURRENT_V8_MANIFEST_SHA256, CatalogProfile,
        CodebaseMemoryIdentityProfile, DATABASE_ACL_SIGNATURE_SQL, EXPECTED_DATABASE_ACL_SIGNATURE,
        EXPECTED_ROLE_SIGNATURE, FUNCTION_ACL_SIGNATURE_SQL, FUNCTION_SIGNATURE_SQL,
        INDEX_SIGNATURE_SQL, RELATION_SIGNATURE_SQL, REQUIRED_APPLICATION_NAME,
        ROLE_DATABASE_BOUNDARY_SQL, ROLE_SIGNATURE_SQL, RetainedHistoryClassification,
        RetainedMigrationHistoryRow, RetainedSchemaCompatibility, SCHEMA_ACL_SIGNATURE_SQL,
        TABLE_ACL_SIGNATURE_SQL, TYPE_CATALOG_SIGNATURE_SQL, V7_AMBIGUITY_COLUMN_SIGNATURE_SQL,
        V7_AMBIGUITY_CONSTRAINT_SIGNATURE_SQL, V7_AMBIGUITY_INDEX_SIGNATURE_SQL,
        V7_AMBIGUITY_RELATION_SIGNATURE_SQL, V7_AMBIGUITY_TABLE_ACL_SIGNATURE_SQL,
        V7_INGRESS_FUNCTION_ACL_SIGNATURE_SQL, V7_INGRESS_FUNCTION_SIGNATURE_SQL,
        WRITER_LEASE_V1_COLUMN_ACL_PROFILE_SQL, WRITER_LEASE_V1_COLUMN_PROFILE_SQL,
        WRITER_LEASE_V1_CONSTRAINT_PROFILE_SQL, WRITER_LEASE_V1_FUNCTION_ACL_PROFILE_SQL,
        WRITER_LEASE_V1_FUNCTION_PROFILE_SQL, WRITER_LEASE_V1_INDEX_PROFILE_SQL,
        WRITER_LEASE_V1_RELATION_PROFILE_SQL, WRITER_LEASE_V1_SCHEMA_ACL_PROFILE_SQL,
        WRITER_LEASE_V1_TABLE_ACL_PROFILE_SQL, WRITER_LEASE_V1_TYPE_PROFILE_SQL, apply_migrations,
        catalog_error, catalog_signature, classify_current_catalog_profile,
        classify_extension_catalog_counts, classify_retained_history_rows,
        codebase_memory_identity_profile_matches, expected_dangerous_function_count,
        expected_internal_trigger_count, expected_owned_function_count,
        expected_scope_head_trigger_count, is_loopback, permission_error, read_database_identity,
        read_forbidden_schema_object_counts, read_history_rows, row_value,
        v3_upgrade_source_has_memory, verify_autonomy_receipt_profile, verify_catalog_signatures,
        verify_cluster_wide_acl_closure, verify_compatibility, verify_effective_default_privileges,
        verify_forbidden_namespace_objects, verify_forbidden_schema_objects, verify_history,
        verify_history_rows, verify_login_principal_closure, verify_network_boundary,
        verify_nonwriter_capabilities, verify_owned_function_acl, verify_owned_function_boundary,
        verify_role_and_database_boundary, verify_roles_and_grants, verify_runtime_store_schema,
        verify_schema_headers, verify_schema_objects, verify_server_version,
        verify_stopped_admission,
    };
    use crate::migrations::{
        DatabaseRole, MigrationTarget, PostgresStoreSetupError, PostgresStoreSetupErrorKind,
        migration_manifest, verify_embedded_manifest, verify_v3_manifest_prefix,
        verify_v6_manifest_prefix,
    };
    use postgres::{Client, NoTls};

    const LIVE_PROFILE_GATE: &str = "LATTICE_STORE_PROFILE_LIVE";
    const LIVE_PROFILE_EXPECTED: &str = "LATTICE_STORE_PROFILE_EXPECTED";
    const LIVE_PROFILE_RUNTIME_URL: &str = "LATTICE_STORE_PROFILE_RUNTIME_URL";
    const LIVE_PROFILE_MIGRATOR_URL: &str = "LATTICE_STORE_PROFILE_MIGRATOR_URL";
    const LIVE_PROFILE_RUN_ID: &str = "LATTICE_TASK019_RUN_ID";

    fn diagnostic_condition(condition: bool) -> Result<(), PostgresStoreSetupError> {
        if condition {
            Ok(())
        } else {
            Err(permission_error())
        }
    }

    fn diagnostic_catalog_condition(condition: bool) -> Result<(), PostgresStoreSetupError> {
        if condition {
            Ok(())
        } else {
            Err(catalog_error())
        }
    }

    fn retained_current_history() -> Vec<RetainedMigrationHistoryRow> {
        migration_manifest()
            .iter()
            .map(|entry| RetainedMigrationHistoryRow {
                ordinal: i16::try_from(entry.ordinal()).expect("ordinal"),
                migration_id: entry.id().to_owned(),
                migration_path: entry.path().to_owned(),
                byte_length: i64::try_from(entry.byte_length()).expect("length"),
                checksum_sha256: entry.sha256().to_owned(),
                migration_status: entry.status().as_str().to_owned(),
                transaction_mode: entry.transaction_mode().as_str().to_owned(),
                schema_version: i16::try_from(entry.schema_version()).expect("schema"),
                min_reader: i16::try_from(*entry.reader_compatibility().start())
                    .expect("min reader"),
                max_reader: i16::try_from(*entry.reader_compatibility().end()).expect("max reader"),
                min_writer: i16::try_from(*entry.writer_compatibility().start())
                    .expect("min writer"),
                max_writer: i16::try_from(*entry.writer_compatibility().end()).expect("max writer"),
            })
            .collect()
    }

    fn compatibility_for(
        rows: &[RetainedMigrationHistoryRow],
        version: i16,
    ) -> RetainedSchemaCompatibility {
        let metadata = super::retained_history_metadata(rows).expect("valid retained metadata");
        RetainedSchemaCompatibility {
            manifest_sha256: crate::migrations::migration_metadata_sha256(&metadata),
            versions: [version; 5],
        }
    }

    #[test]
    fn retained_history_classification_separates_exact_future_and_corrupt_profiles() {
        const FROZEN_CURRENT_V6_MANIFEST_SHA256: &str =
            "75189dea7cd2cb95b694bade467c2b5c40373436fb1b3d48e9017b50a9d206ae";
        let current = retained_current_history();
        let current_compatibility = compatibility_for(&current, 8);
        assert_eq!(
            current_compatibility.manifest_sha256,
            CURRENT_V8_MANIFEST_SHA256
        );
        assert_eq!(
            classify_retained_history_rows(&current, &current_compatibility),
            RetainedHistoryClassification::ExactSupported
        );

        let v6 = current[..7].to_vec();
        let v6_compatibility = compatibility_for(&v6, 6);
        assert_eq!(
            v6_compatibility.manifest_sha256,
            FROZEN_CURRENT_V6_MANIFEST_SHA256
        );
        assert_eq!(
            classify_retained_history_rows(&v6, &v6_compatibility),
            RetainedHistoryClassification::ExactSupported
        );

        let mut future = current.clone();
        future.push(RetainedMigrationHistoryRow {
            ordinal: 11,
            migration_id: "0011_unsupported_fixture".to_owned(),
            migration_path: "db/migrations/0011_unsupported_fixture.sql".to_owned(),
            byte_length: 1,
            checksum_sha256: "d".repeat(64),
            migration_status: "EXECUTABLE".to_owned(),
            transaction_mode: "RUNNER_OWNED".to_owned(),
            schema_version: 9,
            min_reader: 9,
            max_reader: 9,
            min_writer: 9,
            max_writer: 9,
        });
        let future_compatibility = compatibility_for(&future, 9);
        assert_eq!(
            classify_retained_history_rows(&future, &future_compatibility),
            RetainedHistoryClassification::StrictFutureSuffix
        );
        assert_eq!(
            classify_retained_history_rows(&future, &current_compatibility),
            RetainedHistoryClassification::Corrupt
        );

        let mut missing = future.clone();
        missing.remove(3);
        assert_eq!(
            classify_retained_history_rows(&missing, &future_compatibility),
            RetainedHistoryClassification::Corrupt
        );
        let mut reordered = future.clone();
        reordered.swap(2, 3);
        assert_eq!(
            classify_retained_history_rows(&reordered, &future_compatibility),
            RetainedHistoryClassification::Corrupt
        );
        let mut substituted = future.clone();
        substituted[5].checksum_sha256 = "e".repeat(64);
        assert_eq!(
            classify_retained_history_rows(&substituted, &future_compatibility),
            RetainedHistoryClassification::Corrupt
        );
        for invalid_id in ["0011_../forged", "0011_forged/path"] {
            let mut invalid_identity = future.clone();
            invalid_identity[10].migration_id = invalid_id.to_owned();
            invalid_identity[10].migration_path = format!("db/migrations/{invalid_id}.sql");
            let invalid_compatibility = compatibility_for(&invalid_identity, 9);
            assert_eq!(
                classify_retained_history_rows(&invalid_identity, &invalid_compatibility),
                RetainedHistoryClassification::Corrupt
            );
        }
        let mut jumped = future;
        jumped[10].schema_version = 10;
        jumped[10].min_reader = 10;
        jumped[10].max_reader = 10;
        jumped[10].min_writer = 10;
        jumped[10].max_writer = 10;
        let jumped_compatibility = compatibility_for(&jumped, 10);
        assert_eq!(
            classify_retained_history_rows(&jumped, &jumped_compatibility),
            RetainedHistoryClassification::Corrupt
        );
    }

    fn diagnose_forbidden_schema_object(label: &str, result: Result<(), PostgresStoreSetupError>) {
        result
            .unwrap_or_else(|error| panic!("TASK075_CATALOG_DIAGNOSTIC_{label}_{}", error.code()));
        println!("TASK075_CATALOG_DIAGNOSTIC_{label}=PASS");
    }

    fn diagnose_forbidden_schema_objects(client: &mut Client, profile: CatalogProfile) {
        let counts = read_forbidden_schema_object_counts(client).unwrap_or_else(|error| {
            panic!(
                "TASK075_CATALOG_DIAGNOSTIC_FORBIDDEN_OBJECT_QUERY_{}",
                error.code()
            )
        });
        for (label, actual, expected) in [
            (
                "FUNCTION_COUNT",
                counts[0],
                expected_owned_function_count(profile),
            ),
            ("NONINTERNAL_TRIGGER", counts[1], 0),
            ("REWRITE", counts[2], 0),
            ("POLICY", counts[3], 0),
            ("SPECIAL_TYPE", counts[4], 0),
            ("EVENT_TRIGGER", counts[5], 0),
            (
                "SCOPE_TRIGGER",
                counts[6],
                expected_scope_head_trigger_count(profile),
            ),
            (
                "INTERNAL_TRIGGER",
                counts[7],
                expected_internal_trigger_count(profile, false),
            ),
            ("INHERITS", counts[8], 0),
            ("SUBCLASS", counts[9], 0),
        ] {
            diagnose_forbidden_schema_object(
                label,
                diagnostic_catalog_condition(actual == expected),
            );
        }
    }

    fn diagnose_role_boundary(label: &str, result: Result<(), PostgresStoreSetupError>) {
        result
            .unwrap_or_else(|error| panic!("TASK075_CATALOG_DIAGNOSTIC_{label}_{}", error.code()));
        println!("TASK075_CATALOG_DIAGNOSTIC_{label}=PASS");
    }

    fn diagnose_role_and_database_boundary(client: &mut Client, profile: CatalogProfile) {
        diagnose_role_boundary(
            "ROLE_SIGNATURE",
            catalog_signature(
                client,
                ROLE_SIGNATURE_SQL,
                PostgresStoreSetupErrorKind::PermissionDenied,
            )
            .and_then(|actual| diagnostic_condition(actual == EXPECTED_ROLE_SIGNATURE)),
        );
        diagnose_role_boundary(
            "DB_ACL_SIGNATURE",
            catalog_signature(
                client,
                DATABASE_ACL_SIGNATURE_SQL,
                PostgresStoreSetupErrorKind::PermissionDenied,
            )
            .and_then(|actual| diagnostic_condition(actual == EXPECTED_DATABASE_ACL_SIGNATURE)),
        );

        let boundary = client
            .query_one(ROLE_DATABASE_BOUNDARY_SQL, &[])
            .unwrap_or_else(|_| {
                panic!(
                    "TASK075_CATALOG_DIAGNOSTIC_ROLE_BOUNDARY_ROW_STORE_DATABASE_PERMISSION_DENIED"
                )
            });
        diagnose_role_boundary(
            "OWNER",
            row_value::<String>(&boundary, 0, PostgresStoreSetupErrorKind::PermissionDenied)
                .and_then(|actual| diagnostic_condition(actual == DatabaseRole::Migrator.as_str())),
        );
        diagnose_role_boundary(
            "TEMPLATE",
            row_value::<bool>(&boundary, 1, PostgresStoreSetupErrorKind::PermissionDenied)
                .and_then(|actual| diagnostic_condition(!actual)),
        );
        diagnose_role_boundary(
            "ALLOWCONN",
            row_value::<bool>(&boundary, 2, PostgresStoreSetupErrorKind::PermissionDenied)
                .and_then(diagnostic_condition),
        );
        diagnose_role_boundary(
            "CONNLIMIT",
            row_value::<i32>(&boundary, 3, PostgresStoreSetupErrorKind::PermissionDenied)
                .and_then(|actual| diagnostic_condition(actual == -1)),
        );
        diagnose_role_boundary(
            "MEMBERSHIPS",
            row_value::<i64>(&boundary, 4, PostgresStoreSetupErrorKind::PermissionDenied)
                .and_then(|actual| diagnostic_condition(actual == 4)),
        );
        diagnose_role_boundary(
            "EXTRA_ROLES",
            row_value::<i64>(&boundary, 5, PostgresStoreSetupErrorKind::PermissionDenied)
                .and_then(|actual| diagnostic_condition(actual == 0)),
        );
        diagnose_role_boundary(
            "ROLE_SETTINGS",
            row_value::<i64>(&boundary, 6, PostgresStoreSetupErrorKind::PermissionDenied)
                .and_then(|actual| diagnostic_condition(actual == 0)),
        );
        diagnose_role_boundary(
            "DANGEROUS_FUNCTIONS",
            row_value::<i64>(&boundary, 7, PostgresStoreSetupErrorKind::PermissionDenied).and_then(
                |actual| {
                    diagnostic_condition(
                        actual == expected_dangerous_function_count(profile, false),
                    )
                },
            ),
        );
        let database_privileges = (|| {
            Ok([
                row_value::<bool>(&boundary, 8, PostgresStoreSetupErrorKind::PermissionDenied)?,
                row_value::<bool>(&boundary, 9, PostgresStoreSetupErrorKind::PermissionDenied)?,
                row_value::<bool>(&boundary, 10, PostgresStoreSetupErrorKind::PermissionDenied)?,
                row_value::<bool>(&boundary, 11, PostgresStoreSetupErrorKind::PermissionDenied)?,
                row_value::<bool>(&boundary, 12, PostgresStoreSetupErrorKind::PermissionDenied)?,
                row_value::<bool>(&boundary, 13, PostgresStoreSetupErrorKind::PermissionDenied)?,
            ])
        })();
        diagnose_role_boundary(
            "DB_PRIVILEGES",
            database_privileges.and_then(|actual| {
                diagnostic_condition(actual == [false, false, false, true, true, true])
            }),
        );
        diagnose_role_boundary(
            "CLUSTER_ACL",
            verify_cluster_wide_acl_closure(client, profile),
        );
        diagnose_role_boundary("LOGIN_CLOSURE", verify_login_principal_closure(client));
    }

    #[test]
    #[ignore = "requires the coordinated marker-owned disposable PostgreSQL fixture"]
    // Keep signature emission and the matching diagnostic matrix in one
    // ignored fixture entry so a measured digest cannot omit its verifier.
    #[allow(clippy::too_many_lines)]
    fn measure_catalog_signatures() {
        let connection = std::env::var("LATTICE_STORE_CATALOG_SIGNATURE_URL")
            .expect("coordinator supplies fixture URL");
        let mut client = postgres::Client::connect(&connection, postgres::NoTls)
            .expect("connect to coordinated fixture");
        client
            .batch_execute("SET search_path = pg_catalog")
            .expect("harden measurement search path");
        for (label, query) in [
            ("RELATION", RELATION_SIGNATURE_SQL),
            ("COLUMN", COLUMN_SIGNATURE_SQL),
            ("CONSTRAINT", CONSTRAINT_SIGNATURE_SQL),
            ("INDEX", INDEX_SIGNATURE_SQL),
            ("FUNCTION", FUNCTION_SIGNATURE_SQL),
            ("TABLE_ACL", TABLE_ACL_SIGNATURE_SQL),
            ("FUNCTION_ACL", FUNCTION_ACL_SIGNATURE_SQL),
            ("SCHEMA_ACL", SCHEMA_ACL_SIGNATURE_SQL),
            ("AUTONOMY", AUTONOMY_PROFILE_SIGNATURE_SQL),
        ] {
            let signature = catalog_signature(
                &mut client,
                query,
                PostgresStoreSetupErrorKind::CorruptCatalog,
            )
            .expect("catalog signature");
            println!("STORE_CATALOG_{label}_SIGNATURE={signature}");
        }

        let database_name: String = client
            .query_one("SELECT current_database()::text", &[])
            .expect("read fixture database")
            .get(0);
        let run_id = std::env::var(LIVE_PROFILE_RUN_ID).expect("coordinator supplies run id");
        let target = MigrationTarget::new(database_name, run_id).expect("exact fixture target");
        let manifest = verify_embedded_manifest().expect("embedded manifest");
        let profile = classify_current_catalog_profile(&mut client, 5).expect("catalog profile");
        diagnose_forbidden_schema_objects(&mut client, profile);
        for (label, result) in [
            (
                "CATALOG_SIGNATURES",
                verify_catalog_signatures(&mut client, profile, false),
            ),
            (
                "SCHEMA_HEADERS",
                verify_schema_headers(&mut client, profile),
            ),
            (
                "FORBIDDEN_SCHEMA_OBJECTS",
                verify_forbidden_schema_objects(&mut client, profile, false),
            ),
            (
                "FUNCTION_BOUNDARY",
                verify_owned_function_boundary(&mut client, profile, false),
            ),
            (
                "FORBIDDEN_NAMESPACE_OBJECTS",
                verify_forbidden_namespace_objects(&mut client),
            ),
            ("AUTONOMY", verify_autonomy_receipt_profile(&mut client)),
            ("HISTORY", verify_history(&mut client)),
            (
                "COMPATIBILITY",
                verify_compatibility(&mut client, &manifest, profile),
            ),
            (
                "DATABASE_IDENTITY",
                read_database_identity(&mut client, &target).map(|_| ()),
            ),
            ("ADMISSION", verify_stopped_admission(&mut client)),
        ] {
            result.unwrap_or_else(|error| {
                panic!("TASK075_CATALOG_DIAGNOSTIC_{label}_{}", error.code())
            });
            println!("TASK075_CATALOG_DIAGNOSTIC_{label}=PASS");
        }

        diagnose_role_and_database_boundary(&mut client, profile);
        for (label, result) in [
            (
                "ROLE_DATABASE_BOUNDARY",
                verify_role_and_database_boundary(&mut client, profile, false),
            ),
            (
                "FUNCTION_ACL_BOUNDARY",
                verify_owned_function_acl(&mut client, profile),
            ),
            (
                "RUNTIME_CAPABILITIES",
                verify_nonwriter_capabilities(&mut client, DatabaseRole::Runtime, profile),
            ),
            (
                "GUARDIAN_CAPABILITIES",
                verify_nonwriter_capabilities(&mut client, DatabaseRole::Guardian, profile),
            ),
            (
                "READONLY_CAPABILITIES",
                verify_nonwriter_capabilities(&mut client, DatabaseRole::ReadOnly, profile),
            ),
            (
                "DEFAULT_PRIVILEGES",
                verify_effective_default_privileges(&mut client),
            ),
            ("ROLES", verify_roles_and_grants(&mut client, profile)),
        ] {
            result.unwrap_or_else(|error| {
                panic!("TASK075_CATALOG_DIAGNOSTIC_{label}_{}", error.code())
            });
            println!("TASK075_CATALOG_DIAGNOSTIC_{label}=PASS");
        }
    }

    fn emit_owned_catalog_signatures(client: &mut Client, version_label: &str) {
        for (label, query) in [
            ("OWNED_RELATION", RELATION_SIGNATURE_SQL),
            ("OWNED_COLUMN", COLUMN_SIGNATURE_SQL),
            ("OWNED_CONSTRAINT", CONSTRAINT_SIGNATURE_SQL),
            ("OWNED_INDEX", INDEX_SIGNATURE_SQL),
            ("OWNED_FUNCTION", FUNCTION_SIGNATURE_SQL),
            ("OWNED_TYPE", TYPE_CATALOG_SIGNATURE_SQL),
            ("OWNED_TABLE_ACL", TABLE_ACL_SIGNATURE_SQL),
            ("OWNED_FUNCTION_ACL", FUNCTION_ACL_SIGNATURE_SQL),
            ("OWNED_SCHEMA_ACL", SCHEMA_ACL_SIGNATURE_SQL),
        ] {
            let signature =
                catalog_signature(client, query, PostgresStoreSetupErrorKind::CorruptCatalog)
                    .expect("owned catalog signature");
            println!("STORE_{version_label}_CATALOG_{label}_SIGNATURE={signature}");
        }
    }

    fn emit_forbidden_schema_object_counts(client: &mut Client, version_label: &str) {
        let counts =
            read_forbidden_schema_object_counts(client).expect("forbidden schema object counts");
        let encoded = counts
            .iter()
            .map(i64::to_string)
            .collect::<Vec<_>>()
            .join(",");
        println!("STORE_{version_label}_FORBIDDEN_SCHEMA_OBJECT_COUNTS={encoded}");
    }

    fn emit_writer_lease_catalog_signatures(client: &mut Client, version_label: &str) {
        for (label, query) in [
            ("RELATION", WRITER_LEASE_V1_RELATION_PROFILE_SQL),
            ("COLUMN", WRITER_LEASE_V1_COLUMN_PROFILE_SQL),
            ("CONSTRAINT", WRITER_LEASE_V1_CONSTRAINT_PROFILE_SQL),
            ("INDEX", WRITER_LEASE_V1_INDEX_PROFILE_SQL),
            ("FUNCTION", WRITER_LEASE_V1_FUNCTION_PROFILE_SQL),
            ("SCHEMA_ACL", WRITER_LEASE_V1_SCHEMA_ACL_PROFILE_SQL),
            ("TABLE_ACL", WRITER_LEASE_V1_TABLE_ACL_PROFILE_SQL),
            ("FUNCTION_ACL", WRITER_LEASE_V1_FUNCTION_ACL_PROFILE_SQL),
            ("COLUMN_ACL", WRITER_LEASE_V1_COLUMN_ACL_PROFILE_SQL),
            ("TYPE", WRITER_LEASE_V1_TYPE_PROFILE_SQL),
        ] {
            let signature =
                catalog_signature(client, query, PostgresStoreSetupErrorKind::CorruptCatalog)
                    .expect("Writer catalog signature");
            println!("STORE_{version_label}_WRITER_CATALOG_{label}_SIGNATURE={signature}");
        }
    }

    #[test]
    #[ignore = "requires an exact schema-v6 isolated PostgreSQL fixture"]
    fn measure_v6_owned_catalog_signatures() {
        let connection = std::env::var("LATTICE_STORE_V6_CATALOG_SIGNATURE_URL")
            .expect("coordinator supplies schema-v6 fixture URL");
        let mut client = postgres::Client::connect(&connection, postgres::NoTls)
            .expect("connect to schema-v6 fixture");
        client
            .batch_execute("SET search_path = pg_catalog")
            .expect("harden schema-v6 measurement search path");
        let current = client
            .query_one(
                "SELECT current_schema_version,manifest_sha256 \
                   FROM ONLY control.schema_compatibility WHERE singleton=true",
                &[],
            )
            .expect("read exact schema-v6 compatibility");
        let current_schema_version = current.get::<_, i16>(0);
        let current_manifest_sha256 = current.get::<_, String>(1);
        let manifest = verify_v6_manifest_prefix().expect("embedded schema-v6 manifest");
        assert_eq!(current_schema_version, 6, "measurement requires schema-v6");
        assert_eq!(
            current_manifest_sha256,
            manifest.manifest_sha256().as_str(),
            "measurement requires the exact schema-v6 manifest"
        );
        let rows = read_history_rows(&mut client).expect("read schema-v6 history");
        verify_history_rows(&rows, &migration_manifest()[..7]).expect("exact schema-v6 history");
        emit_owned_catalog_signatures(&mut client, "V6");
        emit_forbidden_schema_object_counts(&mut client, "V6");
        emit_writer_lease_catalog_signatures(&mut client, "V6");
    }

    #[test]
    #[ignore = "requires an exact schema-v7 disposable PostgreSQL fixture"]
    fn measure_v7_ingress_signatures() {
        let connection = std::env::var("LATTICE_STORE_V7_CATALOG_SIGNATURE_URL")
            .expect("coordinator supplies schema-v7 fixture URL");
        let mut client = postgres::Client::connect(&connection, postgres::NoTls)
            .expect("connect to schema-v7 fixture");
        client
            .batch_execute("SET search_path = pg_catalog")
            .expect("harden schema-v7 measurement search path");
        let current = client
            .query_one(
                "SELECT current_schema_version,manifest_sha256 \
                   FROM ONLY control.schema_compatibility WHERE singleton=true",
                &[],
            )
            .expect("read exact schema-v7 compatibility");
        let current_schema_version = current.get::<_, i16>(0);
        let current_manifest_sha256 = current.get::<_, String>(1);
        let manifest = verify_embedded_manifest().expect("embedded schema-v7 manifest");
        assert_eq!(current_schema_version, 7, "measurement requires schema-v7");
        assert_eq!(
            current_manifest_sha256,
            manifest.manifest_sha256().as_str(),
            "measurement requires the exact schema-v7 manifest"
        );
        let rows = read_history_rows(&mut client).expect("read schema-v7 history");
        verify_history_rows(&rows, migration_manifest()).expect("exact schema-v7 history");

        emit_owned_catalog_signatures(&mut client, "V7");
        emit_forbidden_schema_object_counts(&mut client, "V7");
        emit_writer_lease_catalog_signatures(&mut client, "V7");
        for (label, query) in [
            ("AMBIGUITY_RELATION", V7_AMBIGUITY_RELATION_SIGNATURE_SQL),
            ("AMBIGUITY_COLUMN", V7_AMBIGUITY_COLUMN_SIGNATURE_SQL),
            (
                "AMBIGUITY_CONSTRAINT",
                V7_AMBIGUITY_CONSTRAINT_SIGNATURE_SQL,
            ),
            ("AMBIGUITY_INDEX", V7_AMBIGUITY_INDEX_SIGNATURE_SQL),
            ("AMBIGUITY_TABLE_ACL", V7_AMBIGUITY_TABLE_ACL_SIGNATURE_SQL),
            ("INGRESS_FUNCTION", V7_INGRESS_FUNCTION_SIGNATURE_SQL),
            (
                "INGRESS_FUNCTION_ACL",
                V7_INGRESS_FUNCTION_ACL_SIGNATURE_SQL,
            ),
        ] {
            let signature = catalog_signature(
                &mut client,
                query,
                PostgresStoreSetupErrorKind::CorruptCatalog,
            )
            .expect("schema-v7 catalog signature");
            println!("STORE_V7_CATALOG_{label}_SIGNATURE={signature}");
        }
    }

    #[test]
    #[ignore = "requires an exact isolated Writer-v4 bridge fixture"]
    fn diagnose_v4_bridge_store_profile() {
        let connection = std::env::var("LATTICE_STORE_V4_BRIDGE_DIAGNOSTIC_URL")
            .expect("coordinator supplies Writer-v4 bridge fixture URL");
        let mut client = postgres::Client::connect(&connection, postgres::NoTls)
            .expect("connect to Writer-v4 bridge fixture");
        client
            .batch_execute("SET search_path = pg_catalog")
            .expect("harden Writer-v4 bridge diagnostic search path");
        super::verify_writer_lease_exact_catalog_profile(
            &mut client,
            &super::WRITER_LEASE_V4_BRIDGE_CATALOG_SIGNATURES,
        )
        .expect("exact Writer-v4 bridge catalog profile");
        println!("STORE_V4_BRIDGE_EXACT_CATALOG_PROFILE=PASS");
        super::verify_writer_lease_v4_functions(&mut client, false)
            .expect("exact Writer-v4 bridge functions");
        println!("STORE_V4_BRIDGE_FUNCTION_PROFILE=PASS");
        super::verify_writer_lease_acl_closure(&mut client, 15, false)
            .expect("exact Writer-v4 bridge ACL closure");
        println!("STORE_V4_BRIDGE_ACL_CLOSURE=PASS");
    }

    #[test]
    fn autonomy_catalog_signature_pins_table_constraints_indexes_and_function_bodies() {
        for required in [
            "pg_catalog.pg_get_constraintdef",
            "pg_catalog.pg_get_indexdef",
            "pg_catalog.pg_get_functiondef",
            "p.prosrc",
            "p.proacl::text",
            "c.relacl::text",
        ] {
            assert!(AUTONOMY_PROFILE_SIGNATURE_SQL.contains(required));
        }
    }

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
            classify_extension_catalog_counts(3, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, false)
                .expect("strict V3"),
            CatalogProfile::V3
        );
        assert_eq!(
            classify_extension_catalog_counts(3, 8, 8, 7, 0, 7, 0, 0, 0, 0, 0, 0, false)
                .expect("exact V3 Memory v2"),
            CatalogProfile::V3CodebaseMemoryV2
        );
        assert_eq!(
            classify_extension_catalog_counts(3, 8, 8, 7, 0, 7, 1, 5, 5, 7, 7, 7, true)
                .expect("exact V3 Memory v2 plus Writer Lease v1"),
            CatalogProfile::V3CodebaseMemoryV2WriterLeaseV1
        );
        assert_eq!(
            format!(
                "{:?}",
                classify_extension_catalog_counts(3, 8, 8, 7, 0, 7, 1, 5, 5, 9, 9, 0, false)
                    .expect("exact V3 Memory v2 plus Writer Lease v2 bridge")
            ),
            "V3CodebaseMemoryV2WriterLeaseV2Bridge"
        );
        assert_eq!(
            classify_extension_catalog_counts(4, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, false)
                .expect("strict V4"),
            CatalogProfile::V4
        );
        assert_eq!(
            classify_extension_catalog_counts(5, 8, 8, 7, 0, 7, 0, 0, 0, 0, 0, 0, false)
                .expect("schema-v5 with exact frozen Memory v2"),
            CatalogProfile::V5CodebaseMemoryV2UpgradePending
        );
        assert_eq!(
            format!(
                "{:?}",
                classify_extension_catalog_counts(5, 8, 8, 7, 0, 7, 1, 5, 5, 9, 9, 0, false)
                    .expect("schema-v5 Memory v2 plus Writer Lease v2 bridge")
            ),
            "V5CodebaseMemoryV2WriterLeaseV2BridgePending"
        );
        assert_eq!(
            classify_extension_catalog_counts(5, 8, 8, 7, 7, 14, 0, 0, 0, 0, 0, 0, false)
                .expect("schema-v5 with exact Memory v3"),
            CatalogProfile::V5CodebaseMemoryV3Current
        );
        assert_eq!(
            format!(
                "{:?}",
                classify_extension_catalog_counts(5, 8, 8, 7, 7, 14, 1, 5, 5, 9, 9, 0, false)
                    .expect("schema-v5 Memory v3 plus Writer Lease v2 bridge")
            ),
            "V5CodebaseMemoryV3WriterLeaseV2BridgePending"
        );
        assert_eq!(
            format!(
                "{:?}",
                classify_extension_catalog_counts(5, 8, 8, 7, 7, 14, 1, 5, 5, 9, 9, 7, true,)
                    .expect("schema-v5 Memory v3 plus current Writer Lease v2")
            ),
            "V5CodebaseMemoryV3WriterLeaseV2Current"
        );
    }

    #[test]
    fn extension_catalog_profile_rejects_unsupported_combinations() {
        assert_eq!(
            classify_extension_catalog_counts(5, 8, 8, 7, 0, 7, 1, 5, 5, 7, 7, 7, true)
                .expect_err("Writer Lease v1 cannot enter a schema-v5 transitional profile")
                .kind(),
            PostgresStoreSetupErrorKind::CorruptCatalog
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
            let writer_present = counts.4 == 1;
            assert_eq!(
                classify_extension_catalog_counts(
                    4,
                    counts.0,
                    counts.1,
                    counts.2,
                    0,
                    counts.3,
                    counts.4,
                    counts.5,
                    counts.6,
                    counts.7,
                    counts.8,
                    if writer_present { 7 } else { 0 },
                    writer_present,
                )
                .expect_err("partial, unknown, extra, or overload must fail")
                .kind(),
                PostgresStoreSetupErrorKind::CorruptCatalog
            );
        }
    }

    #[test]
    fn v3_upgrade_source_preserves_legacy_sources_and_adds_the_exact_writer_bridge() {
        assert!(!v3_upgrade_source_has_memory(CatalogProfile::V3).expect("exact plain V3 source"));
        assert!(
            v3_upgrade_source_has_memory(CatalogProfile::V3CodebaseMemoryV2)
                .expect("exact V3 Memory-v2 source")
        );
        assert!(
            v3_upgrade_source_has_memory(CatalogProfile::V3CodebaseMemoryV2WriterLeaseV2Bridge)
                .expect("exact V3 Memory-v2 Writer-Lease-v2 bridge source")
        );
        for profile in [
            CatalogProfile::V3CodebaseMemoryV2WriterLeaseV1,
            CatalogProfile::V4,
            CatalogProfile::V5,
            CatalogProfile::V5CodebaseMemoryV2UpgradePending,
            CatalogProfile::V5CodebaseMemoryV3Current,
        ] {
            assert_eq!(
                v3_upgrade_source_has_memory(profile)
                    .expect_err("unsupported upgrade source must fail closed")
                    .kind(),
                PostgresStoreSetupErrorKind::CompatibilityMismatch
            );
        }
    }

    #[test]
    fn memory_v3_identity_rejects_every_profile_substitution() {
        let exact = CodebaseMemoryIdentityProfile {
            extension_id: CODEBASE_MEMORY_EXTENSION_ID,
            extension_schema_version: CODEBASE_MEMORY_V3_SCHEMA_VERSION,
            extension_path: CODEBASE_MEMORY_V3_PATH,
            extension_sql_sha256: CODEBASE_MEMORY_V3_SQL_SHA256,
            extension_manifest_sha256: CODEBASE_MEMORY_V3_MANIFEST_SHA256,
            database_uuid: "database-uuid",
            database_identity_sha256: "database-identity",
            global_schema_version: CODEBASE_MEMORY_V3_GLOBAL_SCHEMA_VERSION,
            global_manifest_sha256: "global-manifest",
        };
        assert!(codebase_memory_identity_profile_matches(exact, exact));
        for substituted in [
            CodebaseMemoryIdentityProfile {
                extension_id: "substituted",
                ..exact
            },
            CodebaseMemoryIdentityProfile {
                extension_schema_version: 2,
                ..exact
            },
            CodebaseMemoryIdentityProfile {
                extension_path: "db/extensions/codebase-memory/v2.sql",
                ..exact
            },
            CodebaseMemoryIdentityProfile {
                extension_sql_sha256: "substituted",
                ..exact
            },
            CodebaseMemoryIdentityProfile {
                extension_manifest_sha256: "substituted",
                ..exact
            },
            CodebaseMemoryIdentityProfile {
                database_uuid: "substituted",
                ..exact
            },
            CodebaseMemoryIdentityProfile {
                database_identity_sha256: "substituted",
                ..exact
            },
            CodebaseMemoryIdentityProfile {
                global_schema_version: 3,
                ..exact
            },
            CodebaseMemoryIdentityProfile {
                global_manifest_sha256: "substituted",
                ..exact
            },
        ] {
            assert!(!codebase_memory_identity_profile_matches(
                substituted,
                exact
            ));
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
        if fixture.expected_profile == CatalogProfile::V3CodebaseMemoryV2 {
            assert!(
                v3_upgrade_source_has_memory(fixture.expected_profile)
                    .expect("exact Memory-v2 profile is an eligible V3 upgrade source")
            );
            assert_eq!(assert_live_profile_accepted(&fixture), runtime_identity);
        }
        if fixture.expected_profile == CatalogProfile::V3CodebaseMemoryV2WriterLeaseV1 {
            assert_v3_extension_profile_rejected_as_upgrade_source(&fixture);
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
            "V5" => (CatalogProfile::V5, "V5"),
            "V5_MEMORY_V3" => (CatalogProfile::V5CodebaseMemoryV3Current, "V5_MEMORY_V3"),
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
        eprintln!("TASK075_PROFILE_RUNTIME_CONNECT_ENTER");
        let mut runtime = connect_live_role(&fixture.runtime_url, DatabaseRole::Runtime);
        eprintln!("TASK075_PROFILE_RUNTIME_CONNECT_PASS");
        eprintln!("TASK075_PROFILE_RUNTIME_IDENTITY_ENTER");
        let identity =
            verify_live_connection_identity(&mut runtime, &fixture.target, DatabaseRole::Runtime);
        eprintln!("TASK075_PROFILE_RUNTIME_IDENTITY_PASS");
        eprintln!("TASK075_PROFILE_CLASSIFICATION_ENTER");
        let profile = classify_current_catalog_profile(&mut runtime, 5)
            .unwrap_or_else(|_| panic!("Store live profile classification was rejected"));
        assert_eq!(profile, fixture.expected_profile);
        eprintln!("TASK075_PROFILE_CLASSIFICATION_PASS");
        eprintln!("TASK075_PROFILE_RUNTIME_VERIFY_ENTER");
        verify_runtime_store_schema(&mut runtime, &fixture.target)
            .unwrap_or_else(|error| panic!("TASK075_PROFILE_RUNTIME_VERIFY_{}", error.code()));
        eprintln!("TASK075_PROFILE_RUNTIME_VERIFY_PASS");
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
            .expect_err("V3 Memory plus Writer Lease profile must not be a V5 migration source");
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
